mod support;

use door_pe_loader::image::{self, Export, Import};
use support::{DATA, DLL_BASE, IAT, Image};

fn mapped(fixture: &Image) -> (image::Headers, Vec<u8>) {
    let headers = image::parse_headers(&fixture.bytes).unwrap();
    let mut memory = vec![0u8; headers.size_of_image as usize];
    image::map(&fixture.bytes, &headers, &mut memory).unwrap();
    (headers, memory)
}

#[test]
fn headers_and_sections_are_read_and_laid_out() {
    let fixture = Image::new(true);
    let (headers, memory) = mapped(&fixture);
    assert!(headers.pe32_plus && headers.is_dll());
    assert_eq!(headers.image_base, DLL_BASE);
    assert_eq!(headers.size_of_image, 0x7000);
    assert_eq!(headers.sections().len(), 4);
    assert_eq!(&memory[..2], b"MZ");
    // .text holds ret instructions at its RVA; the gap after it is zero.
    assert_eq!(memory[0x1000], 0xc3);
    assert_eq!(memory[0x1200], 0);
    // Protections: .text RX, .rdata R, .data RW.
    let protections: Vec<u32> = headers.sections().iter().map(|s| s.protection()).collect();
    assert_eq!(protections, [0x20, 0x02, 0x04, 0x02]);
}

#[test]
fn relocation_moves_absolute_pointers_with_the_base() {
    let fixture = Image::new(true);
    let (headers, mut memory) = mapped(&fixture);
    let read =
        |m: &[u8]| u64::from_le_bytes(m[DATA as usize..DATA as usize + 8].try_into().unwrap());
    assert_eq!(read(&memory), DLL_BASE + 0x1010);
    image::relocate(&mut memory, &headers, DLL_BASE).unwrap();
    assert_eq!(read(&memory), DLL_BASE + 0x1010);
    let moved = 0x7ff6_1230_0000;
    image::relocate(&mut memory, &headers, moved).unwrap();
    assert_eq!(read(&memory), moved + 0x1010);
}

#[test]
fn imports_are_bound_by_name_and_ordinal() {
    let mut fixture = Image::new(false);
    fixture.imports("KERNEL32.dll", &["ExitProcess", "GetStdHandle"]);
    let (headers, mut memory) = mapped(&fixture);
    let mut seen = Vec::new();
    image::bind_imports(&mut memory, &headers, |dll, symbol| {
        seen.push((dll.to_vec(), format!("{symbol:?}")));
        Ok::<_, ()>(match symbol {
            Import::Name { name, .. } if name == b"ExitProcess" => 0x7000_1000,
            Import::Name { .. } => 0x7000_2000,
            Import::Ordinal(o) => u64::from(o),
        })
    })
    .unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0, b"KERNEL32.dll");
    let slot = |i: usize| {
        u64::from_le_bytes(
            memory[IAT as usize + i * 8..IAT as usize + i * 8 + 8]
                .try_into()
                .unwrap(),
        )
    };
    assert_eq!((slot(0), slot(1)), (0x7000_1000, 0x7000_2000));
    let mut fixture = Image::new(false);
    fixture.import_ordinal("WS2_32.dll", 115);
    let (headers, mut memory) = mapped(&fixture);
    image::bind_imports(&mut memory, &headers, |_, symbol| {
        assert_eq!(symbol, Import::Ordinal(115));
        Ok::<_, ()>(0x1234)
    })
    .unwrap();
    // A failed resolution stops the walk with the resolver's error.
    let mut fixture = Image::new(false);
    fixture.imports("x.dll", &["Missing"]);
    let (headers, mut memory) = mapped(&fixture);
    assert_eq!(
        image::bind_imports(&mut memory, &headers, |_, _| Err::<u64, _>(127)),
        Err(image::BindError::Resolve(127))
    );
}

#[test]
fn exports_resolve_by_name_hint_and_ordinal() {
    let mut fixture = Image::new(true);
    fixture.exports(&[("Alpha", 0x1000), ("Beta", 0x1010), ("Gamma", 0x1020)]);
    let (headers, memory) = mapped(&fixture);
    let find = |symbol| image::export(&memory, &headers, symbol).unwrap();
    assert_eq!(
        find(Import::Name {
            hint: 0,
            name: b"Beta"
        }),
        Some(Export::Rva(0x1010))
    );
    assert_eq!(
        find(Import::Name {
            hint: 1,
            name: b"Beta"
        }),
        Some(Export::Rva(0x1010))
    );
    assert_eq!(
        find(Import::Name {
            hint: 9,
            name: b"Gamma"
        }),
        Some(Export::Rva(0x1020))
    );
    assert_eq!(
        find(Import::Name {
            hint: 0,
            name: b"Delta"
        }),
        None
    );
    // Ordinal base 1: ordinal 1 is the first function.
    assert_eq!(find(Import::Ordinal(1)), Some(Export::Rva(0x1000)));
    assert_eq!(find(Import::Ordinal(4)), None);
}

#[test]
fn malformed_files_are_refused() {
    assert_eq!(
        image::parse_headers(b"not a pe").unwrap_err(),
        image::Error::Malformed
    );
    let mut fixture = Image::new(true);
    fixture.put16(support::OPTIONAL, 0x10b); // PE32 magic on an x64 machine
    assert_eq!(
        image::parse_headers(&fixture.bytes).unwrap_err(),
        image::Error::Unsupported
    );
}

/// An AnyCPU assembly as the C# compiler writes it: PE32, x86, IL-only,
/// an import of mscoree!_CorDllMain behind a `jmp [IAT]` stub with one
/// HIGHLOW relocation.
fn il_only_assembly(flags: u32) -> Vec<u8> {
    let mut file = vec![0u8; 0x600];
    let put16 =
        |f: &mut Vec<u8>, at: usize, v: u16| f[at..at + 2].copy_from_slice(&v.to_le_bytes());
    let put32 =
        |f: &mut Vec<u8>, at: usize, v: u32| f[at..at + 4].copy_from_slice(&v.to_le_bytes());
    file[..2].copy_from_slice(b"MZ");
    put32(&mut file, 0x3c, 0x80);
    file[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut file, 0x84, 0x14c);
    put16(&mut file, 0x86, 2);
    put16(&mut file, 0x94, 224);
    put16(&mut file, 0x96, 0x2022);
    let o = 0x98;
    put16(&mut file, o, 0x10b);
    put32(&mut file, o + 16, 0x2050);
    put32(&mut file, o + 28, 0x40_0000);
    put32(&mut file, o + 32, 0x2000);
    put32(&mut file, o + 36, 0x200);
    put32(&mut file, o + 56, 0x6000);
    put32(&mut file, o + 60, 0x200);
    put16(&mut file, o + 68, 3);
    put32(&mut file, o + 72, 0x10_0000);
    put32(&mut file, o + 76, 0x1000);
    put32(&mut file, o + 80, 0x10_0000);
    put32(&mut file, o + 84, 0x1000);
    put32(&mut file, o + 92, 16);
    let directory = |f: &mut Vec<u8>, i: usize, rva: u32, size: u32| {
        put32(f, o + 96 + i * 8, rva);
        put32(f, o + 100 + i * 8, size);
    };
    directory(&mut file, 1, 0x2060, 0x28);
    directory(&mut file, 5, 0x4000, 0xc);
    directory(&mut file, 6, 0x2090, 0x1c);
    directory(&mut file, 12, 0x2000, 8);
    directory(&mut file, 14, 0x2008, 0x48);
    let t = o + 224;
    for (i, (rva, raw, chars)) in [
        (0x2000u32, 0x200u32, 0x6000_0020u32),
        (0x4000, 0x400, 0x4200_0040),
    ]
    .into_iter()
    .enumerate()
    {
        let s = t + i * 40;
        file[s..s + 8].copy_from_slice(if i == 0 {
            b".text\0\0\0"
        } else {
            b".reloc\0\0"
        });
        put32(&mut file, s + 8, 0x100);
        put32(&mut file, s + 12, rva);
        put32(&mut file, s + 16, 0x200);
        put32(&mut file, s + 20, raw);
        put32(&mut file, s + 36, chars);
    }
    // COR header at RVA 0x2008 (file 0x208): cb, flags.
    put32(&mut file, 0x208, 0x48);
    put32(&mut file, 0x208 + 16, flags);
    // The x86 stub: jmp dword ptr [0x402000].
    file[0x250..0x252].copy_from_slice(&[0xff, 0x25]);
    put32(&mut file, 0x252, 0x40_2000);
    // One HIGHLOW relocation for the stub's operand.
    put32(&mut file, 0x400, 0x2000);
    put32(&mut file, 0x404, 0xc);
    put16(&mut file, 0x408, 0x3052);
    file
}

#[test]
fn an_anycpu_assembly_maps_as_the_x64_loader_converts_it() {
    let file = il_only_assembly(1);
    let headers = image::parse_headers(&file).unwrap();
    assert!(!headers.pe32_plus && headers.machine == 0x14c);
    let mut memory = vec![0u8; headers.size_of_image as usize];
    image::map(&file, &headers, &mut memory).unwrap();
    assert!(image::is_il_only(&memory, &headers));
    let converted = image::convert_il_only(&mut memory, &headers).unwrap();
    assert!(converted.pe32_plus && converted.machine == 0x14c && converted.is_dll());
    assert_eq!(converted.entry, 0);
    assert_eq!(converted.directory(1), (0, 0));
    assert_eq!(converted.directory(12), (0, 0));
    for kept in [5usize, 6, 14] {
        assert_eq!(converted.directory(kept), headers.directory(kept));
    }
    assert_eq!(converted.sections(), headers.sections());
    assert_eq!(converted.image_base, 0x40_0000);
    assert_eq!(
        (converted.stack_reserve, converted.stack_commit),
        (0x10_0000, 0x1000)
    );
    assert_eq!(u16::from_le_bytes([memory[0x94], memory[0x95]]), 240);
    // Its import is gone, so nothing binds; it still relocates.
    image::bind_imports(&mut memory, &converted, |_, _| Err::<u64, ()>(())).unwrap();
    image::relocate(&mut memory, &converted, 0x7ff6_0000_0000).unwrap();
    assert!(image::is_il_only(&memory, &converted));
}

#[test]
fn a_32_bit_required_assembly_is_not_anycpu() {
    for flags in [3u32, 0] {
        let file = il_only_assembly(flags);
        let headers = image::parse_headers(&file).unwrap();
        let mut memory = vec![0u8; headers.size_of_image as usize];
        image::map(&file, &headers, &mut memory).unwrap();
        assert!(!image::is_il_only(&memory, &headers));
    }
}
