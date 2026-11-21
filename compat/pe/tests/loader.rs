mod support;

use door_pe_loader::{
    BundlePlan, Feature, FixupKind, ImageBase, ImageError, Limit, ModuleSource, Permissions,
    Reason, Symbol, VisitError,
};
use support::*;

fn reason(sources: &[ModuleSource<'_>]) -> Reason {
    BundlePlan::new(sources).unwrap_err().reason
}

#[test]
fn resolves_real_dependency_order_and_streams_relocated_imports() {
    let mut exe = Image::new(false);
    exe.imports("KeRnEl32.DlL", &["WriteFile"]);
    let mut kernel = Image::new(true);
    kernel.exports(&[("WriteFile", 0x3400)]);
    kernel.string(0x3400, "ntdll.#1");
    let mut nt = Image::new(true);
    nt.exports(&[("NtWriteFile", 0x1000)]);
    let selected = EXE_BASE + 0x100_0000;
    let plan = BundlePlan::new(&[
        exe.source("work.exe", selected),
        kernel.source("kernel32.dll", DLL_BASE),
        nt.source("ntdll.dll", DLL_BASE + 0x100_0000),
    ])
    .unwrap();
    assert_eq!(plan.dependency_order(), [2, 1, 0]);
    assert_eq!(plan.entry().0, selected + 0x1000);
    assert_eq!(
        plan.resolve_export(1, Symbol::Name("WriteFile")).unwrap().0,
        DLL_BASE + 0x100_1000
    );
    let mut fixups = Vec::new();
    plan.visit_fixups(|fixup| {
        fixups.push(fixup);
        Ok::<_, ()>(())
    })
    .unwrap();
    assert_eq!(fixups.len(), 4);
    let import = fixups
        .iter()
        .find(|fixup| fixup.kind == FixupKind::Import)
        .unwrap();
    assert_eq!(import.address.0, selected + u64::from(IAT));
    assert_eq!(import.value, DLL_BASE + 0x100_1000);
    assert_eq!(
        fixups
            .iter()
            .find(|f| f.module == 0 && f.kind == FixupKind::Dir64)
            .unwrap()
            .value,
        selected + 0x1010
    );
    assert!(matches!(fixups.last().unwrap().kind, FixupKind::Import));
}

#[test]
fn mapping_covers_full_image_and_leaves_holes_reserved_without_access() {
    let mut image = Image::new(false);
    image.directory(5, 0, 0);
    image.put32(OPTIONAL + 16, 0x2000);
    image.put32(OPTIONAL + 56, 0xc000);
    for (index, rva) in [0x2000, 0x4000, 0x8000, 0xa000].into_iter().enumerate() {
        image.put32(SECTION_TABLE + index * 40 + 12, rva);
    }
    let plan = BundlePlan::new(&[image.source("work.exe", EXE_BASE)]).unwrap();
    let mut cursor = EXE_BASE;
    let mut holes = 0;
    let mut runs = Vec::new();
    plan.visit_runs(|run| {
        assert_eq!(run.address.0, cursor);
        assert_eq!(run.bytes % 4096, 0);
        cursor += u64::from(run.bytes);
        if run.permissions == Permissions::None {
            holes += 1;
            assert!(run.file.is_none());
        }
        runs.push(run);
        Ok::<_, ()>(())
    })
    .unwrap();
    assert_eq!(cursor, EXE_BASE + 0xc000);
    assert_eq!(holes, 5);
    assert_eq!(runs[0].permissions, Permissions::Read);
    assert_eq!(runs[0].file.unwrap().bytes, 0x400);
    assert_eq!(runs[2].permissions, Permissions::ReadExecute);
}

#[test]
fn zero_fill_is_described_without_borrowing_bytes_beyond_file() {
    let mut image = Image::new(false);
    image.put32(SECTION_TABLE + 2 * 40 + 8, 0x1000);
    let plan = BundlePlan::new(&[image.source("work.exe", EXE_BASE)]).unwrap();
    let mut data = None;
    plan.visit_runs(|run| {
        if run.rva.0 == DATA {
            data = Some(run);
        }
        Ok::<_, ()>(())
    })
    .unwrap();
    let data = data.unwrap();
    assert_eq!(data.bytes, 4096);
    assert_eq!(data.file.unwrap().bytes, 512);
    assert_eq!(data.permissions, Permissions::ReadWrite);
}

#[test]
fn imports_by_ordinal_and_separate_original_thunks_resolve() {
    let mut exe = Image::new(false);
    exe.import_ordinal("kernel32.dll", 1);
    exe.rva32(0x2000, 0x2800);
    exe.rva64(0x2800, (1 << 63) | 1);
    let mut dll = Image::new(true);
    dll.exports(&[("Entry", 0x1000)]);
    assert!(
        BundlePlan::new(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("kernel32.dll", DLL_BASE)
        ])
        .is_ok()
    );
    exe.rva64(IAT, (1 << 63) | 2);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("kernel32.dll", DLL_BASE)
        ]),
        Reason::InvalidImport
    );
}

#[test]
fn missing_module_missing_symbol_and_ordinal_holes_are_precise() {
    let mut exe = Image::new(false);
    exe.imports("kernel32.dll", &["WriteFile"]);
    assert_eq!(
        reason(&[exe.source("work.exe", EXE_BASE)]),
        Reason::MissingModule
    );
    let mut dll = Image::new(true);
    dll.exports(&[("Other", 0x1000)]);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("kernel32.dll", DLL_BASE)
        ]),
        Reason::MissingSymbol
    );
    exe.import_ordinal("kernel32.dll", 2);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("kernel32.dll", DLL_BASE)
        ]),
        Reason::MissingSymbol
    );
    // An unnamed ordinal hole is valid metadata but cannot resolve.
    dll.rva32(0x3000 + 24, 0);
    dll.rva32(0x3040, 0);
    exe.import_ordinal("kernel32.dll", 1);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("kernel32.dll", DLL_BASE)
        ]),
        Reason::MissingSymbol
    );
}

#[test]
fn unsafe_names_duplicates_and_mismatched_personalities_fail_closed() {
    let mut exe = Image::new(false);
    let dll = Image::new(true);
    for name in [
        "../evil.dll",
        "C:evil.dll",
        "evil/sub.dll",
        "\\evil.dll",
        "evil..dll",
    ] {
        exe.imports(name, &["Entry"]);
        assert_eq!(
            reason(&[
                exe.source("work.exe", EXE_BASE),
                dll.source("safe.dll", DLL_BASE)
            ]),
            Reason::InvalidModuleName
        );
    }
    let exe = Image::new(false);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("same.dll", DLL_BASE),
            dll.source("SAME.DLL", DLL_BASE + 0x10000)
        ]),
        Reason::DuplicateModule
    );
    assert_eq!(
        reason(&[dll.source("work.exe", EXE_BASE)]),
        Reason::WrongImageKind
    );
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            exe.source("wrong.dll", DLL_BASE)
        ]),
        Reason::WrongImageKind
    );
}

#[test]
fn module_cycles_and_forwarder_cycles_are_bounded() {
    let mut exe = Image::new(false);
    exe.imports("a.dll", &["Entry"]);
    let mut a = Image::new(true);
    let mut b = Image::new(true);
    a.exports(&[("Entry", 0x1000)]);
    b.exports(&[("Entry", 0x1000)]);
    a.imports("b.dll", &["Entry"]);
    b.imports("a.dll", &["Entry"]);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            a.source("a.dll", DLL_BASE),
            b.source("b.dll", DLL_BASE + 0x10000)
        ]),
        Reason::DependencyCycle
    );
    a.directory(1, 0, 0);
    a.exports(&[("Entry", 0x3400)]);
    a.string(0x3400, "a.Entry");
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            a.source("a.dll", DLL_BASE)
        ]),
        Reason::ForwarderCycle
    );
}

#[test]
fn forwarder_bounds_missing_destinations_and_callable_data_checks() {
    let exe = Image::new(false);
    let mut dll = Image::new(true);
    dll.exports(&[("Entry", 0x3400)]);
    for text in ["missing.Entry", "missing.#1"] {
        dll.string(0x3400, text);
        assert_eq!(
            reason(&[
                exe.source("work.exe", EXE_BASE),
                dll.source("a.dll", DLL_BASE)
            ]),
            Reason::MissingModule
        );
    }
    for text in ["a.#65536", "a.#", "a.", "a.#-1"] {
        dll.string(0x3400, text);
        assert_eq!(
            reason(&[
                exe.source("work.exe", EXE_BASE),
                dll.source("a.dll", DLL_BASE)
            ]),
            Reason::InvalidExport
        );
    }
    dll.exports(&[("Entry", DATA)]);
    let plan = BundlePlan::new(&[
        exe.source("work.exe", EXE_BASE),
        dll.source("a.dll", DLL_BASE),
    ])
    .unwrap();
    assert_eq!(
        plan.resolve_export(1, Symbol::Name("Entry")).unwrap().0,
        DLL_BASE + u64::from(DATA)
    );
    assert_eq!(
        plan.resolve_executable_export(1, Symbol::Name("Entry"))
            .unwrap_err()
            .reason,
        Reason::InvalidExport
    );
}

#[test]
fn address_overflow_kernel_addresses_and_full_image_collisions_reject() {
    let exe = Image::new(false);
    for base in [0, 4096, EXE_BASE + 1, !65535, 0xffff_8000_0000_0000] {
        assert_eq!(
            reason(&[exe.source("work.exe", base)]),
            Reason::InvalidAddress
        );
    }
    let mut exe = Image::new(false);
    exe.put32(OPTIONAL + 56, 0x20000); // reserved tail also owns its address range.
    let dll = Image::new(true);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("a.dll", EXE_BASE + 0x10000)
        ]),
        Reason::AddressCollision
    );
}

#[test]
fn relocation_absence_stripping_types_order_and_targets_reject() {
    let mut exe = Image::new(false);
    exe.directory(5, 0, 0);
    assert!(BundlePlan::new(&[exe.source("work.exe", EXE_BASE)]).is_ok());
    assert_eq!(
        reason(&[exe.source("work.exe", EXE_BASE + 0x10000)]),
        Reason::RelocationRequired
    );
    exe.relocations(&[0xa000, 0]);
    exe.put16(0x96, 0x23);
    assert_eq!(
        reason(&[exe.source("work.exe", EXE_BASE + 0x10000)]),
        Reason::RelocationRequired
    );
    exe.put16(0x96, 0x22);
    for entries in [[0x3000, 0], [0xafff, 0], [0xa000, 0xa000], [0xa008, 0xa000]] {
        exe.relocations(&entries);
        assert!(BundlePlan::new(&[exe.source("work.exe", EXE_BASE + 0x10000)]).is_err());
    }
    exe.relocations(&[0xa000, 0]);
    exe.rva64(DATA, 0xffff_8000_0000_1000);
    assert_eq!(
        reason(&[exe.source("work.exe", EXE_BASE + 0x10000)]),
        Reason::InvalidRelocation
    );
    exe.rva64(DATA, EXE_BASE + 0x1000);
    exe.rva32(RELOC + 4, u32::MAX);
    assert_eq!(
        reason(&[exe.source("work.exe", EXE_BASE)]),
        Reason::InvalidRelocation
    );
}

#[test]
fn a_relocation_may_cross_a_page_inside_its_section() {
    // Windows applies a record at any 12-bit offset: one at 0xFFC of a
    // page writes into the next page of the same section (.rdata here).
    let mut exe = Image::new(false);
    exe.directory(5, RELOC, 12);
    exe.rva32(RELOC, 0x2000);
    exe.rva32(RELOC + 4, 12);
    exe.rva16(RELOC + 8, 0xaffc);
    exe.rva16(RELOC + 10, 0);
    exe.rva64(0x2ffc, EXE_BASE + 0x1010);
    let base = EXE_BASE + 0x10000;
    let plan = BundlePlan::new(&[exe.source("work.exe", base)]).unwrap();
    let mut fixups = Vec::new();
    plan.visit_fixups(|fixup| {
        fixups.push((fixup.rva.0, fixup.value, fixup.width));
        Ok::<(), ()>(())
    })
    .unwrap();
    assert!(fixups.contains(&(0x2ffc, base + 0x1010, 8)));
    // A record whose target leaves its section is still refused.
    exe.rva32(RELOC, 0x4000);
    exe.rva16(RELOC + 8, 0xaffc);
    assert!(BundlePlan::new(&[exe.source("work.exe", base)]).is_err());
}

#[test]
fn overlapping_relocation_and_iat_patches_are_not_order_dependent() {
    let mut exe = Image::new(false);
    exe.imports("a.dll", &["Entry"]);
    // With preferred base0, the name thunk is also a structurally valid
    // relocatable image pointer. It must still never overlap the IAT fixup.
    exe.put64(OPTIONAL + 24, 0);
    exe.rva64(DATA, 0x1010);
    exe.rva32(RELOC, 0x2000);
    exe.rva16(RELOC + 8, 0xa400);
    let mut dll = Image::new(true);
    dll.exports(&[("Entry", 0x1000)]);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("a.dll", DLL_BASE)
        ]),
        Reason::OverlappingFixups
    );
    // Two descriptors may not share IAT storage either.
    exe.relocations(&[0xa000, 0]);
    exe.directory(1, 0x2000, 60);
    let descriptor = exe.bytes[exe.offset(0x2000)..exe.offset(0x2000) + 20].to_vec();
    let offset = exe.offset(0x2014);
    exe.bytes[offset..offset + 20].copy_from_slice(&descriptor);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("a.dll", DLL_BASE)
        ]),
        Reason::OverlappingFixups
    );
}

#[test]
fn required_features_never_disappear_as_empty_metadata() {
    for (directory, feature) in [
        (2, Feature::Resources),
        (9, Feature::Tls),
        (10, Feature::LoadConfiguration),
        (11, Feature::BoundImports),
        (14, Feature::ManagedImage),
    ] {
        let mut exe = Image::new(false);
        exe.directory(directory, 0x3800, 4);
        assert_eq!(
            reason(&[exe.source("work.exe", EXE_BASE)]),
            Reason::Unsupported(feature)
        );
    }
    let mut exe = Image::new(false);
    exe.directory(13, 0x3800, 4);
    assert_eq!(
        reason(&[exe.source("work.exe", EXE_BASE)]),
        Reason::Image(ImageError::Unsupported)
    );
    let mut dll = Image::new(true);
    dll.put32(OPTIONAL + 16, 0x1000);
    assert_eq!(
        reason(&[
            Image::new(false).source("work.exe", EXE_BASE),
            dll.source("a.dll", DLL_BASE)
        ]),
        Reason::Unsupported(Feature::DllInitialization)
    );
    let mut exe = Image::new(false);
    exe.put16(OPTIONAL + 70, 0x4160);
    assert_eq!(
        reason(&[exe.source("work.exe", EXE_BASE)]),
        Reason::Unsupported(Feature::SecurityPolicy)
    );
}

#[test]
fn unwind_extents_opcodes_and_required_handlers_are_validated() {
    let mut exe = Image::new(false);
    exe.unwind([1, 8, 1, 0], &[0x4208]); // alloc-small 40 bytes at prolog offset8.
    assert!(BundlePlan::new(&[exe.source("work.exe", EXE_BASE)]).is_ok());
    for (header, codes, expected) in [
        (
            [9, 8, 1, 0],
            vec![0x4208],
            Reason::Unsupported(Feature::UnwindHandler),
        ),
        (
            [2, 8, 1, 0],
            vec![0x4208],
            Reason::Unsupported(Feature::UnwindVersion),
        ),
        ([1, 8, 1, 0], vec![0x0108], Reason::InvalidUnwind),
        ([1, 8, 1, 0], vec![0x4209], Reason::InvalidUnwind),
        ([1, 8, 1, 5], vec![0x4208], Reason::InvalidUnwind),
        ([1, 8, 1, 0], vec![0x0608], Reason::InvalidUnwind),
    ] {
        exe.unwind(header, &codes);
        assert_eq!(reason(&[exe.source("work.exe", EXE_BASE)]), expected);
    }
    exe.unwind([1, 8, 1, 0], &[0x4208]);
    exe.rva32(0x3808, 0x4ffc);
    assert!(BundlePlan::new(&[exe.source("work.exe", EXE_BASE)]).is_err());
}

#[test]
fn bounded_resources_include_gap_regions_and_import_terminators() {
    let exe = Image::new(false);
    let dll = Image::new(true);
    let sources = [
        exe.source("work.exe", EXE_BASE),
        dll.source("a.dll", DLL_BASE),
        dll.source("b.dll", DLL_BASE + 0x10000),
        dll.source("c.dll", DLL_BASE + 0x20000),
        dll.source("d.dll", DLL_BASE + 0x30000),
    ];
    assert_eq!(sources.len(), 5); // Initial policy retains its four-module cap.
    assert_eq!(reason(&sources), Reason::ResourceLimit(Limit::Modules));
    let mut exe = Image::new(false);
    exe.put32(OPTIONAL + 56, 0x1001000);
    assert_eq!(
        reason(&[exe.source("work.exe", EXE_BASE)]),
        Reason::ResourceLimit(Limit::ImageBytes)
    );
    let mut exe = Image::new(false);
    exe.import_ordinal("a.dll", 1);
    for index in 0..=1024 {
        exe.rva64(IAT + index * 8, (1 << 63) | 1);
    }
    let mut dll = Image::new(true);
    dll.exports(&[("Entry", 0x1000)]);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("a.dll", DLL_BASE)
        ]),
        Reason::ResourceLimit(Limit::Imports)
    );
}

#[test]
fn construction_is_pure_and_apply_failure_preserves_exact_callback_error() {
    let image = Image::new(false);
    let original = image.bytes.clone();
    let plan = BundlePlan::new(&[image.source("work.exe", EXE_BASE + 0x10000)]).unwrap();
    for fail_at in 0..5 {
        let mut owned = Vec::new();
        let result = plan.visit_runs(|run| {
            if owned.len() == fail_at {
                return Err(73_u32);
            }
            owned.push(run);
            Ok(())
        });
        assert_eq!(result, Err(VisitError::Callback(73)));
        assert_eq!(owned.len(), fail_at);
        owned.clear(); // caller-owned rollback, no unpublished reservation remains.
        assert!(owned.is_empty());
    }
    assert_eq!(
        plan.visit_fixups(|_| Err(91_u32)),
        Err(VisitError::Callback(91))
    );
    assert_eq!(image.bytes, original);
}

#[test]
fn truncated_headers_overlapping_sections_and_wx_are_rejected() {
    let image = Image::new(false);
    for end in [0, 1, 63, 64, 128, 255, 511, image.bytes.len() - 1] {
        let source = ModuleSource {
            name: "work.exe",
            bytes: &image.bytes[..end],
            base: ImageBase(EXE_BASE),
        };
        assert!(BundlePlan::new(&[source]).is_err(), "truncation at {end}");
    }
    let mut image = Image::new(false);
    image.put32(SECTION_TABLE + 2 * 40 + 12, 0x3000);
    assert_eq!(
        reason(&[image.source("work.exe", EXE_BASE)]),
        Reason::Image(ImageError::Overlap)
    );
    let mut image = Image::new(false);
    image.put32(SECTION_TABLE + 36, 0xe000_0020);
    assert_eq!(
        reason(&[image.source("work.exe", EXE_BASE)]),
        Reason::Image(ImageError::Permissions)
    );
}

#[test]
fn immutable_plan_stays_small_enough_for_kernel_construction_stack() {
    // Thirty-two module slots (P18: .NET programs load most of the catalog)
    // on a 64 KiB trusted syscall stack: the plan must stay a small fraction
    // of it alongside the 16 KiB Windows state page.
    let bytes = core::mem::size_of::<BundlePlan<'_>>();
    assert!(bytes < 6144, "plan is {bytes} bytes");
}

#[test]
fn readable_data_and_zero_fill_exports_resolve_but_never_become_callable() {
    let mut exe = Image::new(false);
    exe.imports("a.dll", &["Datum"]);
    for target in [0x2a00, DATA, DATA + 0x300] {
        let mut dll = Image::new(true);
        dll.put32(SECTION_TABLE + 2 * 40 + 8, 0x1000);
        dll.exports(&[("Datum", target)]);
        let plan = BundlePlan::new(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("a.dll", DLL_BASE),
        ])
        .unwrap();
        assert_eq!(
            plan.resolve_export(1, Symbol::Name("Datum")).unwrap().0,
            DLL_BASE + u64::from(target)
        );
        assert_eq!(
            plan.resolve_executable_export(1, Symbol::Name("Datum"))
                .unwrap_err()
                .reason,
            Reason::InvalidExport
        );
        let mut imported = 0;
        plan.visit_fixups(|fixup| {
            if fixup.kind == FixupKind::Import {
                imported = fixup.value;
            }
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(imported, DLL_BASE + u64::from(target));
    }
    for target in [0, 0x5800, 0x8000, u32::MAX] {
        let mut dll = Image::new(true);
        dll.exports(&[("Datum", target)]);
        assert!(
            BundlePlan::new(&[
                exe.source("work.exe", EXE_BASE),
                dll.source("a.dll", DLL_BASE)
            ])
            .is_err()
        );
    }
    for flags in [0, 0x2000_0000, 0x8000_0000] {
        let mut dll = Image::new(true);
        dll.exports(&[("Datum", DATA)]);
        dll.put32(SECTION_TABLE + 2 * 40 + 36, flags);
        assert_eq!(
            reason(&[
                exe.source("work.exe", EXE_BASE),
                dll.source("a.dll", DLL_BASE)
            ]),
            Reason::InvalidLayout
        );
    }
}

#[test]
fn reservation_run_and_section_limits_are_enforced_before_mapping() {
    fn gaps(image: &mut Image) {
        image.directory(5, 0, 0);
        if image.base == EXE_BASE {
            image.put32(OPTIONAL + 16, 0x2000);
        }
        image.put32(OPTIONAL + 56, 0xc000);
        for (index, rva) in [0x2000, 0x4000, 0x8000, 0xa000].into_iter().enumerate() {
            image.put32(SECTION_TABLE + index * 40 + 12, rva);
        }
    }
    let mut exe = Image::new(false);
    let mut a = Image::new(true);
    let mut b = Image::new(true);
    gaps(&mut exe);
    gaps(&mut a);
    gaps(&mut b);
    b.put64(OPTIONAL + 24, DLL_BASE + 0x10000);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            a.source("a.dll", DLL_BASE),
            b.source("b.dll", DLL_BASE + 0x10000)
        ]),
        Reason::ResourceLimit(Limit::Runs)
    );
    let mut image = Image::new(false);
    for index in 0..16 {
        image.directory(index, 0, 0);
    }
    image.put16(0x86, 9);
    image.put32(OPTIONAL + 56, 0xa000);
    for index in 0..9 {
        let section = SECTION_TABLE + index * 40;
        image.put32(section + 8, 0x200);
        image.put32(section + 12, (index + 1) as u32 * 0x1000);
        image.put32(section + 16, 0x200);
        image.put32(section + 20, 0x400 + index as u32 * 0x200);
        image.put32(
            section + 36,
            if index == 0 { 0x6000_0020 } else { 0x4000_0040 },
        );
    }
    assert_eq!(
        reason(&[image.source("work.exe", EXE_BASE)]),
        Reason::ResourceLimit(Limit::Sections)
    );
}

#[test]
fn malformed_ordinal_bits_and_unsorted_export_names_are_rejected() {
    let mut exe = Image::new(false);
    exe.import_ordinal("a.dll", 1);
    let mut dll = Image::new(true);
    dll.exports(&[("A", 0x1000), ("B", 0x1010)]);
    exe.rva64(IAT, 0x8000_0001_0000_0001);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("a.dll", DLL_BASE)
        ]),
        Reason::InvalidImport
    );
    exe.import_ordinal("a.dll", 1);
    dll.exports(&[("B", 0x1000), ("A", 0x1010)]);
    assert_eq!(
        reason(&[
            exe.source("work.exe", EXE_BASE),
            dll.source("a.dll", DLL_BASE)
        ]),
        Reason::InvalidExport
    );
}

#[test]
fn malformed_unwind_function_ranges_and_chains_are_rejected() {
    let mut image = Image::new(false);
    image.unwind([1, 8, 1, 0], &[0x4208]);
    image.directory(3, 0x3800, 24);
    image.rva32(0x380c, 0x1080);
    image.rva32(0x3810, 0x1180);
    image.rva32(0x3814, 0x3900);
    assert_eq!(
        reason(&[image.source("work.exe", EXE_BASE)]),
        Reason::InvalidUnwind
    );
    image.unwind([0x21, 8, 1, 0], &[0x4208]);
    assert_eq!(
        reason(&[image.source("work.exe", EXE_BASE)]),
        Reason::Unsupported(Feature::UnwindHandler)
    );
    image.unwind([1, 8, 1, 0], &[0x4208]);
    image.rva32(0x3804, 0x1400); // executable padding is not backed function code.
    assert_eq!(
        reason(&[image.source("work.exe", EXE_BASE)]),
        Reason::InvalidUnwind
    );
}

#[test]
fn mutated_images_never_panic_or_emit_out_of_reservation_fixups() {
    let mut original = Image::new(false);
    original.imports("a.dll", &["Entry"]);
    original.unwind([1, 8, 1, 0], &[0x4208]);
    let mut dll = Image::new(true);
    dll.exports(&[("Entry", 0x1000)]);
    let mut state = 0x0019_d07e_ab51_u64;
    for iteration in 0..20_000 {
        let mut bytes = original.bytes.clone();
        for _ in 0..(iteration % 4 + 1) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let offset = state as usize % bytes.len();
            bytes[offset] = (state >> 24) as u8;
        }
        if let Ok(plan) = BundlePlan::new(&[
            ModuleSource {
                name: "work.exe",
                bytes: &bytes,
                base: ImageBase(EXE_BASE + 0x10000),
            },
            dll.source("a.dll", DLL_BASE),
        ]) {
            let mut runs = 0;
            plan.visit_runs(|run| {
                let module = plan.module(run.module).unwrap();
                assert!(run.address.0 >= module.source.base.0);
                assert!(
                    run.address.0 + u64::from(run.bytes)
                        <= module.source.base.0 + u64::from(module.image_bytes())
                );
                if let Some(file) = run.file {
                    assert!(
                        u64::from(file.offset.0) + u64::from(file.bytes)
                            <= module.source.bytes.len() as u64
                    );
                }
                runs += 1;
                Ok::<_, ()>(())
            })
            .unwrap();
            assert!(runs <= door_pe_loader::MAX_RUNS);
            plan.visit_fixups(|fixup| {
                let module = plan.module(fixup.module).unwrap();
                assert!(fixup.address.0 >= module.source.base.0);
                assert!(
                    fixup.address.0 + 8 <= module.source.base.0 + u64::from(module.image_bytes())
                );
                Ok::<_, ()>(())
            })
            .unwrap();
        }
    }
}
