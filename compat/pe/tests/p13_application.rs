mod support;

use door_pe_loader::{
    BundlePlan, DEFAULT_SECURITY_COOKIE, Feature, FixupKind, ImageBase, ImageError, Limit,
    LoadPolicy, ModuleSource, Reason, closure::import_closure, resources,
};
use support::*;

const RSRC: u32 = 0x4000;
const LOAD_CONFIG: u32 = 0x4600;
const DELAY: u32 = 0x4800;

fn app<'a>(sources: &[ModuleSource<'a>]) -> Result<BundlePlan<'a>, Reason> {
    BundlePlan::with_policy(sources, LoadPolicy::Application).map_err(|e| e.reason)
}

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Three-level tree: "TEXT"/"README" in 0x407 and 0x409, MENU #1 in 0x409.
fn resource_tree(image: &mut Image) {
    image.directory(2, RSRC, 0xd8);
    let put_entry = |image: &mut Image, at: u32, name: u32, target: u32| {
        image.rva32(RSRC + at, name);
        image.rva32(RSRC + at + 4, target);
    };
    let table = |image: &mut Image, at: u32, named: u16, ids: u16| {
        image.rva16(RSRC + at + 12, named);
        image.rva16(RSRC + at + 14, ids);
    };
    let string = |image: &mut Image, at: u32, text: &str| {
        image.rva16(RSRC + at, text.len() as u16);
        for (index, unit) in text.encode_utf16().enumerate() {
            image.rva16(RSRC + at + 2 + index as u32 * 2, unit);
        }
    };
    table(image, 0x00, 1, 1);
    put_entry(image, 0x10, 0x8000_0020, 0x8000_0030);
    put_entry(image, 0x18, 4, 0x8000_0058);
    string(image, 0x20, "TEXT");
    table(image, 0x30, 1, 0);
    put_entry(image, 0x40, 0x8000_0048, 0x8000_0070);
    string(image, 0x48, "README");
    table(image, 0x58, 0, 1);
    put_entry(image, 0x68, 1, 0x8000_0090);
    table(image, 0x70, 0, 2);
    put_entry(image, 0x80, 0x407, 0xa8);
    put_entry(image, 0x88, 0x409, 0xb8);
    table(image, 0x90, 0, 1);
    put_entry(image, 0xa0, 0x409, 0xc8);
    for (at, rva, size) in [(0xa8, 0x4400, 5), (0xb8, 0x4410, 7), (0xc8, 0x4420, 12)] {
        image.rva32(RSRC + at, rva);
        image.rva32(RSRC + at + 4, size);
        image.rva32(RSRC + at + 8, 1252);
    }
}

#[test]
fn gui_subsystem_is_application_only_and_other_subsystems_stay_refused() {
    let mut exe = Image::new(false);
    exe.put16(OPTIONAL + 68, 2);
    let inputs = [exe.source("app.exe", EXE_BASE)];
    assert_eq!(
        BundlePlan::new(&inputs).unwrap_err().reason,
        Reason::Unsupported(Feature::Subsystem)
    );
    assert_eq!(
        BundlePlan::with_policy(&inputs, LoadPolicy::DllCallbacks)
            .unwrap_err()
            .reason,
        Reason::Unsupported(Feature::Subsystem)
    );
    let plan = BundlePlan::with_policy(&inputs, LoadPolicy::Application).unwrap();
    assert_eq!(plan.subsystem(), 2);
    for subsystem in [0, 1, 7, 9, 10, 16] {
        exe.put16(OPTIONAL + 68, subsystem);
        assert_eq!(
            app(&[exe.source("app.exe", EXE_BASE)]).unwrap_err(),
            Reason::Unsupported(Feature::Subsystem)
        );
    }
    let mut dll = Image::new(true);
    dll.put16(OPTIONAL + 68, 2);
    let exe = Image::new(false);
    assert!(
        app(&[
            exe.source("app.exe", EXE_BASE),
            dll.source("gui.dll", DLL_BASE)
        ])
        .is_ok()
    );
}

#[test]
fn resources_resolve_by_id_name_and_windows_language_fallback() {
    let mut exe = Image::new(false);
    resource_tree(&mut exe);
    assert_eq!(
        BundlePlan::new(&[exe.source("app.exe", EXE_BASE)])
            .unwrap_err()
            .reason,
        Reason::Unsupported(Feature::Resources)
    );
    let plan = app(&[exe.source("app.exe", EXE_BASE)]).unwrap();
    let module = plan.module(0).unwrap();
    assert_eq!(module.resource_directory().unwrap().1, 0xd8);
    let (text, readme) = (utf16("text"), utf16("ReadMe"));
    let found = |kind, name, language| module.find_resource(kind, name, language);
    use resources::{Error, Id};
    // Neutral request on the US profile prefers 0x409.
    let entry = found(Id::Name(&text), Id::Name(&readme), 0).unwrap();
    assert_eq!(
        (entry.data_rva, entry.size, entry.code_page),
        (0x4410, 7, 1252)
    );
    assert_eq!(entry.offset, 0xb8);
    let entry = found(Id::Name(&text), Id::Name(&readme), 0x407).unwrap();
    assert_eq!(entry.data_rva, 0x4400);
    assert_eq!(
        found(Id::Name(&text), Id::Name(&readme), 0x40c),
        Err(Error::LanguageNotFound)
    );
    let menu = found(Id::Integer(resources::kind::MENU), Id::Integer(1), 0).unwrap();
    assert_eq!((menu.data_rva, menu.size), (0x4420, 12));
    assert_eq!(
        found(Id::Integer(resources::kind::DIALOG), Id::Integer(1), 0),
        Err(Error::TypeNotFound)
    );
    assert_eq!(
        found(Id::Integer(resources::kind::MENU), Id::Integer(2), 0),
        Err(Error::NameNotFound)
    );
    // A name never matches an integer identity and vice versa.
    assert_eq!(
        found(Id::Name(&utf16("4")), Id::Integer(1), 0),
        Err(Error::TypeNotFound)
    );
    // Neutral with only a non-profile language falls back to the first entry.
    let mut german = Image::new(false);
    resource_tree(&mut german);
    german.rva32(RSRC + 0x88, 0x40c);
    let plan = app(&[german.source("app.exe", EXE_BASE)]).unwrap();
    assert_eq!(
        plan.module(0)
            .unwrap()
            .find_resource(Id::Name(&text), Id::Name(&readme), 0)
            .unwrap()
            .data_rva,
        0x4400
    );
    let without = Image::new(false);
    let plan = app(&[without.source("app.exe", EXE_BASE)]).unwrap();
    assert_eq!(
        plan.module(0)
            .unwrap()
            .find_resource(Id::Integer(1), Id::Integer(1), 0),
        Err(Error::NoData)
    );
}

#[test]
fn malformed_resource_trees_are_refused_before_mapping() {
    let cases: [(u32, u32); 9] = [
        (0xa8, 0x9000),          // data outside the image
        (0xa8, 0x0fff),          // data in the header page
        (0xac, 0x10_0000),       // size crossing the section
        (0x40 + 4, 0x8000_0000), // subdirectory loops to the root
        (0x80 + 4, 0x8000_0070), // fourth level: language names a directory
        (0x18 + 4, 0x0000_00c8), // type level names a data entry
        (0x20, 0),               // empty string name
        (14, 0x2000),            // entry count beyond the directory
        (0x10, 0x8000_0fff),     // misaligned string beyond the extent
    ];
    for (at, value) in cases {
        let mut exe = Image::new(false);
        resource_tree(&mut exe);
        if at % 4 == 2 {
            exe.rva16(RSRC + at, value as u16);
        } else {
            exe.rva32(RSRC + at, value);
        }
        let reason = app(&[exe.source("app.exe", EXE_BASE)]).unwrap_err();
        assert!(
            matches!(
                reason,
                Reason::InvalidResource | Reason::ResourceLimit(Limit::Resources)
            ),
            "case {at:#x}: {reason:?}"
        );
    }
    // Named/ID ordering of entries must agree with the table counts.
    let mut exe = Image::new(false);
    resource_tree(&mut exe);
    exe.rva16(RSRC + 12, 0);
    exe.rva16(RSRC + 14, 2);
    assert_eq!(
        app(&[exe.source("app.exe", EXE_BASE)]).unwrap_err(),
        Reason::InvalidResource
    );
}

#[test]
fn shared_subdirectories_cannot_amplify_resource_validation_work() {
    // 200 type entries all naming one 200-entry name table whose entries share
    // one language table: 40,000 leaves if expanded. Validation stops at the
    // global visit budget instead.
    let mut exe = Image::new(false);
    let root = 0x2000;
    let types = 200u32;
    let names = 0x660;
    let languages = 0xcc0;
    let leaf = 0xcd8;
    exe.directory(2, root, 0xce8);
    exe.rva16(root + 14, types as u16);
    for index in 0..types {
        exe.rva32(root + 16 + index * 8, index + 1);
        exe.rva32(root + 20 + index * 8, 0x8000_0000 | names);
    }
    exe.rva16(root + names + 14, types as u16);
    for index in 0..types {
        exe.rva32(root + names + 16 + index * 8, index + 1);
        exe.rva32(root + names + 20 + index * 8, 0x8000_0000 | languages);
    }
    exe.rva16(root + languages + 14, 1);
    exe.rva32(root + languages + 16, 0x409);
    exe.rva32(root + languages + 20, leaf);
    exe.rva32(root + leaf, 0x1000);
    exe.rva32(root + leaf + 4, 1);
    assert_eq!(
        app(&[exe.source("app.exe", EXE_BASE)]).unwrap_err(),
        Reason::ResourceLimit(Limit::Resources)
    );
    // The same tree with one type is an ordinary valid directory.
    exe.rva16(root + 14, 1);
    exe.rva16(root + names + 14, 1);
    assert!(app(&[exe.source("app.exe", EXE_BASE)]).is_ok());
}

fn load_config(image: &mut Image) {
    image.directory(10, LOAD_CONFIG, 0x140);
    image.rva32(LOAD_CONFIG, 0x140);
    image.rva64(LOAD_CONFIG + 88, image.base + u64::from(DATA + 0x100));
    image.rva64(DATA + 0x100, DEFAULT_SECURITY_COOKIE);
    // CFG check/dispatch slots point at writable image data, as the CRT does.
    image.rva64(LOAD_CONFIG + 112, image.base + u64::from(DATA + 0x110));
    image.rva64(LOAD_CONFIG + 120, image.base + u64::from(DATA + 0x118));
}

#[test]
fn load_configuration_honours_the_security_cookie() {
    let mut exe = Image::new(false);
    load_config(&mut exe);
    assert_eq!(
        BundlePlan::new(&[exe.source("app.exe", EXE_BASE)])
            .unwrap_err()
            .reason,
        Reason::Unsupported(Feature::LoadConfiguration)
    );
    let selected = EXE_BASE + 0x10_0000;
    let plan = app(&[exe.source("app.exe", selected)]).unwrap();
    let config = plan.module(0).unwrap().load_config().unwrap().unwrap();
    assert_eq!(config.security_cookie.unwrap().0, DATA + 0x100);
    let cookie = |seed: Option<u64>| {
        let mut cookies = Vec::new();
        let mut collect = |fixup: door_pe_loader::Fixup| {
            if fixup.kind == FixupKind::SecurityCookie {
                cookies.push((fixup.address.0, fixup.value));
            }
            Ok::<_, ()>(())
        };
        match seed {
            Some(seed) => plan.visit_fixups_with_cookie(seed, &mut collect).unwrap(),
            None => plan.visit_fixups(&mut collect).unwrap(),
        }
        cookies
    };
    assert_eq!(
        cookie(None),
        [(selected + u64::from(DATA + 0x100), DEFAULT_SECURITY_COOKIE)]
    );
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..512u64 {
        let [(address, value)] = cookie(Some(seed))[..] else {
            panic!("one cookie");
        };
        assert_eq!(address, selected + u64::from(DATA + 0x100));
        assert!(value != 0 && value != DEFAULT_SECURITY_COOKIE && value >> 48 == 0);
        seen.insert(value);
    }
    assert_eq!(seen.len(), 512, "seeds diversify the cookie");
    // An image that deliberately ships its own value keeps it.
    exe.rva64(DATA + 0x100, 0x1234);
    let plan = app(&[exe.source("app.exe", selected)]).unwrap();
    let mut rewritten = 0;
    plan.visit_fixups_with_cookie(7, |fixup| {
        rewritten += usize::from(fixup.kind == FixupKind::SecurityCookie);
        Ok::<_, ()>(())
    })
    .unwrap();
    assert_eq!(rewritten, 0);
}

#[test]
fn load_configuration_refuses_unsupported_or_escaping_fields() {
    for (offset, value, expected) in [
        (96, 1, Reason::Unsupported(Feature::LegacyTables)),
        (104, 1, Reason::Unsupported(Feature::LegacyTables)),
        (40, 1, Reason::Unsupported(Feature::LegacyTables)),
        (192, 1, Reason::Unsupported(Feature::DynamicRelocations)),
        (200, 1, Reason::Unsupported(Feature::HybridImage)),
        (248, 1, Reason::Unsupported(Feature::Enclave)),
        (148, 1, Reason::Unsupported(Feature::CodeIntegrity)),
        (112, 0xffff_8000_0000_0000, Reason::InvalidLoadConfiguration),
        (88, EXE_BASE + 0x2000, Reason::InvalidLoadConfiguration), // read-only cookie
        (
            88,
            EXE_BASE + u64::from(DATA + 0x104),
            Reason::InvalidLoadConfiguration,
        ),
        (128, EXE_BASE + 0x2000, Reason::InvalidLoadConfiguration), // count without table
    ] {
        let mut exe = Image::new(false);
        load_config(&mut exe);
        if offset == 128 {
            exe.rva64(LOAD_CONFIG + 128, 0);
            exe.rva64(LOAD_CONFIG + 136, 4);
        } else if offset == 148 {
            exe.rva16(LOAD_CONFIG + 148, value as u16);
        } else {
            exe.rva64(LOAD_CONFIG + offset, value);
        }
        assert_eq!(
            app(&[exe.source("app.exe", EXE_BASE)]).unwrap_err(),
            expected,
            "field {offset}"
        );
    }
    for size in [0, 4, 0x2000] {
        let mut exe = Image::new(false);
        load_config(&mut exe);
        exe.rva32(LOAD_CONFIG, size);
        assert_eq!(
            app(&[exe.source("app.exe", EXE_BASE)]).unwrap_err(),
            Reason::InvalidLoadConfiguration
        );
    }
    // A relocation can never target the cookie the loader rewrites.
    let mut exe = Image::new(false);
    load_config(&mut exe);
    exe.relocations(&[0xa000, 0xa100]);
    exe.rva64(DATA + 0x100, 0);
    assert!(matches!(
        app(&[exe.source("app.exe", EXE_BASE)]).unwrap_err(),
        Reason::OverlappingFixups | Reason::InvalidRelocation
    ));
    // P18: a CFG-instrumented image loads as on a Windows not enforcing
    // CFG (its own check/dispatch slots stay); earlier policies refuse it.
    let mut exe = Image::new(false);
    load_config(&mut exe);
    exe.put16(OPTIONAL + 70, 0x4160);
    assert!(app(&[exe.source("app.exe", EXE_BASE)]).is_ok());
    assert_eq!(
        BundlePlan::new(&[exe.source("app.exe", EXE_BASE)])
            .map(|_| ())
            .map_err(|e| e.reason),
        Err(Reason::Unsupported(Feature::SecurityPolicy))
    );
}

fn delay_import(image: &mut Image, module: &str, symbol: &str) {
    image.directory(13, DELAY, 64);
    image.rva32(DELAY, 1);
    image.rva32(DELAY + 4, 0x4980);
    image.rva32(DELAY + 8, DATA + 0x80);
    image.rva32(DELAY + 12, DATA + 0x40);
    image.rva32(DELAY + 16, 0x4880);
    image.rva64(0x4880, 0x4900);
    image.rva64(0x4888, 0);
    image.rva16(0x4900, 0);
    image.string(0x4902, symbol);
    image.string(0x4980, module);
    // The delay IAT initially points at the image's own helper thunk.
    image.rva64(DATA + 0x40, image.base + 0x1000);
    image.relocations(&[0xa000, 0xa040]);
}

#[test]
fn delay_imports_bind_eagerly_with_checked_slots() {
    let mut exe = Image::new(false);
    delay_import(&mut exe, "late.dll", "Entry");
    let mut late = Image::new(true);
    late.exports(&[("Entry", 0x1000)]);
    let inputs = [
        exe.source("app.exe", EXE_BASE),
        late.source("late.dll", DLL_BASE),
    ];
    assert_eq!(
        BundlePlan::new(&inputs).unwrap_err().reason,
        Reason::Image(ImageError::Unsupported)
    );
    let plan = app(&inputs).unwrap();
    let mut fixups = Vec::new();
    plan.visit_fixups(|fixup| {
        fixups.push(fixup);
        Ok::<_, ()>(())
    })
    .unwrap();
    let handle = fixups
        .iter()
        .find(|f| f.kind == FixupKind::ModuleHandle)
        .unwrap();
    assert_eq!(
        (handle.address.0, handle.value),
        (EXE_BASE + u64::from(DATA + 0x80), DLL_BASE)
    );
    // The relocation of the helper thunk precedes the import write.
    let slot = EXE_BASE + u64::from(DATA + 0x40);
    let writes: Vec<_> = fixups.iter().filter(|f| f.address.0 == slot).collect();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].kind, FixupKind::Dir64);
    assert_eq!(
        (writes[1].kind, writes[1].value),
        (FixupKind::Import, DLL_BASE + 0x1000)
    );
    // A delay-only edge never orders initialization, so cycles are accepted.
    let mut a = Image::new(true);
    a.exports(&[("Entry", 0x1000)]);
    delay_import(&mut a, "b.dll", "Entry");
    let mut b = Image::new(true);
    b.exports(&[("Entry", 0x1000)]);
    b.imports("a.dll", &["Entry"]);
    let mut exe = Image::new(false);
    exe.imports("b.dll", &["Entry"]);
    let plan = app(&[
        exe.source("app.exe", EXE_BASE),
        a.source("a.dll", DLL_BASE),
        b.source("b.dll", DLL_BASE + 0x10000),
    ])
    .unwrap();
    assert_eq!(plan.dependency_order(), [1, 2, 0]);
    // ...but a deferred module still cannot satisfy a startup delay import.
    assert_eq!(
        plan.validate_deferred_module(2).unwrap_err().reason,
        Reason::DeferredDependency
    );
}

#[test]
fn malformed_delay_descriptors_are_refused() {
    let late = {
        let mut late = Image::new(true);
        late.exports(&[("Entry", 0x1000)]);
        late
    };
    let check = |exe: &Image| {
        app(&[
            exe.source("app.exe", EXE_BASE),
            late.source("late.dll", DLL_BASE),
        ])
        .unwrap_err()
    };
    let mut exe = Image::new(false);
    delay_import(&mut exe, "missing.dll", "Entry");
    assert_eq!(check(&exe), Reason::MissingModule);
    let mut exe = Image::new(false);
    delay_import(&mut exe, "late.dll", "Absent");
    assert_eq!(check(&exe), Reason::MissingSymbol);
    for (at, value, expected) in [
        (DELAY, 0, Reason::Unsupported(Feature::LegacyDelayImports)),
        (DELAY, 3, Reason::InvalidDelayImport),
        (DELAY + 8, 0x2000, Reason::InvalidDelayImport), // read-only HMODULE slot
        (DELAY + 8, DATA + 0x84, Reason::InvalidDelayImport),
        (DELAY + 12, 0, Reason::InvalidDelayImport),
        (DELAY + 16, 0x9000, Reason::Image(ImageError::Bounds)),
        (DELAY + 4, 0x9000, Reason::InvalidDelayImport),
        (DELAY + 12, IAT, Reason::OverlappingFixups),
    ] {
        let mut exe = Image::new(false);
        exe.imports("late.dll", &["Entry"]);
        delay_import(&mut exe, "late.dll", "Entry");
        exe.rva32(at, value);
        let reason = check(&exe);
        assert!(
            reason == expected
                || (matches!(expected, Reason::Image(_)) && matches!(reason, Reason::Image(_))),
            "field {at:#x}={value:#x}: {reason:?}"
        );
    }
    // Missing terminator beyond the declared descriptors: unbounded tables fail.
    let mut exe = Image::new(false);
    delay_import(&mut exe, "late.dll", "Entry");
    for index in 1..40u32 {
        let base = DELAY + index * 32;
        for word in 0..8 {
            let value =
                exe.bytes[exe.offset(DELAY + word * 4)..exe.offset(DELAY + word * 4) + 4].to_vec();
            let offset = exe.offset(base + word * 4);
            exe.bytes[offset..offset + 4].copy_from_slice(&value);
        }
    }
    assert!(matches!(
        check(&exe),
        Reason::ResourceLimit(Limit::DelayImports) | Reason::OverlappingFixups
    ));
}

/// Image with several import descriptors (each IAT 64 bytes apart).
fn importing(dll: bool, imports: &[(&str, &str)]) -> Image {
    let mut image = Image::new(dll);
    image.directory(1, 0x2000, (imports.len() as u32 + 1) * 20);
    for (index, (module, symbol)) in imports.iter().enumerate() {
        let index = index as u32;
        let descriptor = 0x2000 + index * 20;
        let iat = 0x2400 + index * 64;
        let name = 0x2100 + index * 32;
        let hint = 0x2200 + index * 32;
        image.rva32(descriptor, iat);
        image.rva32(descriptor + 12, name);
        image.rva32(descriptor + 16, iat);
        image.string(name, module);
        image.rva16(hint, 0);
        image.string(hint + 2, symbol);
        image.rva64(iat, u64::from(hint));
        image.rva64(iat + 8, 0);
    }
    image
}

#[test]
fn bundle_is_the_import_closure_over_the_fixed_catalog() {
    let mut nt = Image::new(true);
    nt.exports(&[("NtWriteFile", 0x1000)]);
    let mut kernel = Image::new(true);
    kernel.exports(&[("WriteFile", 0x3400)]);
    kernel.string(0x3400, "ntdll.NtWriteFile");
    let mut user = importing(
        true,
        &[("kernel32.dll", "WriteFile"), ("gdi32.dll", "Draw")],
    );
    user.exports(&[("CreateWindowExW", 0x1000)]);
    let mut gdi = Image::new(true);
    gdi.exports(&[("Draw", 0x1000)]);
    let mut dialogs = Image::new(true);
    dialogs.exports(&[("GetOpenFileNameW", 0x1000)]);
    let unused = Image::new(true);
    let catalog = [
        nt.source("ntdll.dll", DLL_BASE),
        kernel.source("kernel32.dll", DLL_BASE + 0x10000),
        user.source("user32.dll", DLL_BASE + 0x20000),
        gdi.source("gdi32.dll", DLL_BASE + 0x30000),
        dialogs.source("comdlg32.dll", DLL_BASE + 0x40000),
        unused.source("comctl32.dll", DLL_BASE + 0x50000),
    ];
    let mut exe = importing(
        false,
        &[
            ("API-MS-WIN-CORE-FILE-L1-1-0.DLL", "WriteFile"),
            ("USER32.dll", "CreateWindowExW"),
        ],
    );
    let closure = import_closure(
        exe.source("app.exe", EXE_BASE),
        &catalog,
        &[0],
        LoadPolicy::Application,
    )
    .unwrap();
    assert_eq!(closure.providers(), [0, 1, 2, 3]);
    delay_import(&mut exe, "comdlg32.dll", "GetOpenFileNameW");
    let closure = import_closure(
        exe.source("app.exe", EXE_BASE),
        &catalog,
        &[0],
        LoadPolicy::Application,
    )
    .unwrap();
    assert_eq!(closure.providers(), [0, 1, 2, 3, 4]);
    let mut bundle = vec![exe.source("app.exe", EXE_BASE)];
    bundle.extend(closure.providers().iter().map(|&index| catalog[index]));
    let plan = app(&bundle).unwrap();
    assert_eq!(plan.module_count(), 6);
    // An unknown dependency is reported against its importer, never searched.
    let user = importing(true, &[("winmm.dll", "PlaySoundW")]);
    let mut catalog = catalog;
    catalog[2] = user.source("user32.dll", DLL_BASE + 0x20000);
    assert_eq!(
        import_closure(
            exe.source("app.exe", EXE_BASE),
            &catalog,
            &[0],
            LoadPolicy::Application
        )
        .unwrap_err(),
        door_pe_loader::Error {
            module: 3,
            rva: Some(door_pe_loader::Rva(0)),
            reason: Reason::MissingModule
        }
    );
}

#[test]
fn import_closure_caps_the_export_scan_at_the_ordinal_space() {
    let nt = Image::new(true);
    let exe = importing(false, &[("gdi32.dll", "Draw")]);
    let closure = |base: u32, functions: u32| {
        let mut gdi = Image::new(true);
        gdi.exports(&[("Draw", 0x1000)]);
        gdi.rva32(0x3000 + 16, base);
        gdi.rva32(0x3000 + 20, functions);
        let catalog = [
            nt.source("ntdll.dll", DLL_BASE),
            gdi.source("gdi32.dll", DLL_BASE + 0x10000),
        ];
        import_closure(
            exe.source("app.exe", EXE_BASE),
            &catalog,
            &[0],
            LoadPolicy::Application,
        )
        .map(|closure| closure.providers().to_vec())
    };
    // Ordinal space past 65,536 (or overflowing u32) is refused before the
    // export address table is walked, as the bundle plan refuses it.
    for (base, functions) in [(65536, 1), (65535, 2), (u32::MAX, 1)] {
        assert_eq!(
            closure(base, functions),
            Err(door_pe_loader::Error {
                module: 2,
                rva: Some(door_pe_loader::Rva(0x3000)),
                reason: Reason::InvalidExport
            }),
            "base {base} functions {functions}"
        );
    }
    // The last legal ordinal still resolves.
    assert_eq!(closure(65535, 1), Ok(vec![0, 1]));
    // A function count beyond the image parser's table cap never reaches
    // the scan.
    assert_eq!(
        closure(1, 65536).unwrap_err().reason,
        Reason::Image(door_pe_loader::ImageError::Limit)
    );
}

#[test]
fn application_limits_fit_a_gui_bundle_with_headroom() {
    let names: Vec<String> = (0..door_pe_loader::MAX_MODULES - 1)
        .map(|i| format!("m{i}.dll"))
        .collect();
    let dlls: Vec<Image> = (0..door_pe_loader::MAX_MODULES - 1)
        .map(|_| {
            let mut dll = Image::new(true);
            dll.exports(&[("Entry", 0x1000)]);
            dll
        })
        .collect();
    let exe = Image::new(false);
    let mut sources = vec![exe.source("app.exe", EXE_BASE)];
    for (index, dll) in dlls.iter().enumerate() {
        sources.push(dll.source(&names[index], DLL_BASE + index as u64 * 0x10000));
    }
    assert_eq!(sources.len(), door_pe_loader::MAX_MODULES);
    let plan = app(&sources).unwrap();
    assert_eq!(plan.module_count(), door_pe_loader::MAX_MODULES);
    let mut runs = 0;
    plan.visit_runs(|_| {
        runs += 1;
        Ok::<_, ()>(())
    })
    .unwrap();
    assert!(runs <= door_pe_loader::MAX_RUNS);
    let extra = Image::new(true);
    sources.push(extra.source("extra.dll", DLL_BASE + 0x100_0000));
    assert_eq!(
        app(&sources).unwrap_err(),
        Reason::ResourceLimit(Limit::Modules)
    );
    // The qualified P10 profile keeps its five-module cap.
    assert_eq!(
        BundlePlan::with_policy(&sources[..6], LoadPolicy::DllCallbacks)
            .unwrap_err()
            .reason,
        Reason::ResourceLimit(Limit::Modules)
    );
}

fn tls_dll(base: u64) -> Image {
    let mut dll = Image::new(true);
    dll.put64(OPTIONAL + 24, base);
    dll.directory(5, 0, 0);
    dll.directory(9, 0x2800, 40);
    dll.rva64(0x2800, base + u64::from(DATA + 32));
    dll.rva64(0x2808, base + u64::from(DATA + 40));
    dll.rva64(0x2810, base + u64::from(DATA + 16));
    dll.rva64(0x2818, 0);
    dll
}

#[test]
fn static_tls_modules_are_bounded_and_get_stable_slots() {
    let exe = Image::new(false);
    let dlls: Vec<Image> = (0..6).map(|i| tls_dll(DLL_BASE + i * 0x10000)).collect();
    let plain = Image::new(true);
    let names: Vec<String> = (0..6).map(|i| format!("t{i}.dll")).collect();
    let mut sources = vec![
        exe.source("app.exe", EXE_BASE),
        plain.source("plain.dll", DLL_BASE + 0x100000),
    ];
    for (index, dll) in dlls.iter().enumerate().take(5) {
        sources.push(dll.source(&names[index], DLL_BASE + index as u64 * 0x10000));
    }
    let plan = app(&sources).unwrap();
    assert_eq!(plan.tls_slot(0), None);
    assert_eq!(plan.tls_slot(1), None);
    assert_eq!(plan.tls_slot(2), Some(0));
    assert_eq!(plan.tls_slot(6), Some(4));
    sources.push(dlls[5].source(&names[5], DLL_BASE + 5 * 0x10000));
    assert_eq!(
        app(&sources).unwrap_err(),
        Reason::ResourceLimit(Limit::TlsModules)
    );
}

#[test]
fn mutated_application_images_never_panic_or_escape_their_reservations() {
    let mut original = Image::new(false);
    original.put16(OPTIONAL + 68, 2);
    resource_tree(&mut original);
    load_config(&mut original);
    delay_import(&mut original, "late.dll", "Entry");
    let mut late = Image::new(true);
    late.exports(&[("Entry", 0x1000)]);
    assert!(
        app(&[
            original.source("app.exe", EXE_BASE),
            late.source("late.dll", DLL_BASE)
        ])
        .is_ok()
    );
    // Concentrate mutations on the headers and the P13 metadata regions.
    let hot = [
        (0usize, 0x400usize),
        (original.offset(RSRC), 0xd8),
        (original.offset(LOAD_CONFIG), 0x140),
        (original.offset(DELAY), 0x200),
        (original.offset(DATA), 0x200),
    ];
    let mut state = 0x5eed_1234_abcd_u64;
    let mut accepted = 0;
    for iteration in 0..20_000 {
        let mut bytes = original.bytes.clone();
        for _ in 0..(iteration % 4 + 1) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let (start, length) = hot[(state >> 40) as usize % hot.len()];
            let offset = start + state as usize % length;
            bytes[offset] = (state >> 24) as u8;
        }
        let sources = [
            ModuleSource {
                name: "app.exe",
                bytes: &bytes,
                base: ImageBase(EXE_BASE + 0x10000),
            },
            late.source("late.dll", DLL_BASE),
        ];
        let Ok(plan) = BundlePlan::with_policy(&sources, LoadPolicy::Application) else {
            continue;
        };
        accepted += 1;
        let module = plan.module(0).unwrap();
        plan.visit_fixups_with_cookie(state, |fixup| {
            let owner = plan.module(fixup.module).unwrap();
            assert!(fixup.address.0 >= owner.source.base.0);
            assert!(fixup.address.0 + 8 <= owner.source.base.0 + u64::from(owner.image_bytes()));
            Ok::<_, ()>(())
        })
        .unwrap();
        for kind in [resources::kind::MENU, resources::kind::STRING] {
            if let Ok(entry) =
                module.find_resource(resources::Id::Integer(kind), resources::Id::Integer(1), 0)
            {
                assert!(
                    u64::from(entry.data_rva) + u64::from(entry.size)
                        <= u64::from(module.image_bytes())
                );
            }
        }
        let _ = module.find_resource(
            resources::Id::Name(&utf16("TEXT")),
            resources::Id::Name(&utf16("README")),
            0x409,
        );
    }
    assert!(accepted > 1000, "mutations still exercise accepted plans");
}

#[test]
fn application_images_may_carry_language_handler_frames_but_not_chains() {
    // Ordinary MSVC code and the pinned precompiled Rust core/alloc mark
    // frames with exception/termination handlers (__CxxFrameHandler3).
    for flags in [0x09u8, 0x11, 0x19] {
        let mut exe = Image::new(false);
        exe.unwind([flags, 8, 1, 0], &[0x4208]);
        exe.rva32(0x3908, 0x1050); // handler RVA after the padded code array
        let inputs = [exe.source("work.exe", EXE_BASE)];
        assert!(app(&inputs).is_ok(), "flags {flags:#x}");
        // The earlier policies still refuse every handler frame.
        assert_eq!(
            BundlePlan::new(&inputs).map(|_| ()).map_err(|e| e.reason),
            Err(Reason::Unsupported(Feature::UnwindHandler))
        );
        // A handler outside the module's executable code is malformed.
        exe.rva32(0x3908, 0x3800);
        assert_eq!(
            app(&[exe.source("work.exe", EXE_BASE)]).map(|_| ()),
            Err(Reason::InvalidUnwind)
        );
    }
    // P18: chained infos (the unwinder follows them) are accepted under
    // the application policy; the earlier policies still refuse them.
    let mut chained = Image::new(false);
    chained.unwind([0x21, 8, 1, 0], &[0x4208]);
    chained.rva32(0x3908, 0x1000);
    chained.rva32(0x390c, 0x1010);
    chained.rva32(0x3910, 0x3920);
    let primary = chained.offset(0x3920);
    chained.bytes[primary..primary + 4].copy_from_slice(&[1, 0, 0, 0]);
    assert!(app(&[chained.source("work.exe", EXE_BASE)]).is_ok());
    assert_eq!(
        BundlePlan::new(&[chained.source("work.exe", EXE_BASE)])
            .map(|_| ())
            .map_err(|e| e.reason),
        Err(Reason::Unsupported(Feature::UnwindHandler))
    );
    // A chain into itself, outside executable code, or with handler flags
    // is malformed.
    for (at, value) in [(0x3910, 0x3900), (0x3908, 0x3800)] {
        let mut bad = chained.clone();
        bad.rva32(at, value);
        assert_eq!(
            app(&[bad.source("work.exe", EXE_BASE)]).map(|_| ()),
            Err(Reason::InvalidUnwind)
        );
    }
    let mut bad = chained.clone();
    bad.unwind([0x29, 8, 1, 0], &[0x4208]);
    assert_eq!(
        app(&[bad.source("work.exe", EXE_BASE)]).map(|_| ()),
        Err(Reason::InvalidUnwind)
    );
}

#[test]
fn application_images_may_carry_version_2_epilogue_codes() {
    // MSVC's CRT (libcmt, libvcruntime) emits version 2 with UWOP_EPILOG.
    let mut exe = Image::new(false);
    exe.unwind([2, 8, 3, 0], &[0x1601, 0x0000, 0x4208]);
    assert!(app(&[exe.source("work.exe", EXE_BASE)]).is_ok());
    assert_eq!(
        BundlePlan::new(&[exe.source("work.exe", EXE_BASE)])
            .map(|_| ())
            .map_err(|e| e.reason),
        Err(Reason::Unsupported(Feature::UnwindVersion))
    );
    // A two-slot code cut short, an unknown code and version 3 are refused.
    for (header, codes, expected) in [
        ([2, 8, 1, 0], vec![0x1601], Reason::InvalidUnwind),
        ([2, 8, 1, 0], vec![0x0b00], Reason::InvalidUnwind),
        (
            [3, 8, 1, 0],
            vec![0x4208],
            Reason::Unsupported(Feature::UnwindVersion),
        ),
    ] {
        exe.unwind(header, &codes);
        assert_eq!(
            app(&[exe.source("work.exe", EXE_BASE)]).map(|_| ()),
            Err(expected)
        );
    }
}

#[test]
fn aslr_images_without_relocations_move_only_under_the_application_policy() {
    // MSVC omits the base relocation directory of an x64 /DYNAMICBASE image
    // with no absolute addresses; Windows maps it at any base.
    let mut exe = Image::new(false);
    exe.directory(5, 0, 0);
    let moved = EXE_BASE + 0x200_0000;
    assert!(app(&[exe.source("work.exe", moved)]).is_ok());
    assert_eq!(
        BundlePlan::new(&[exe.source("work.exe", moved)])
            .map(|_| ())
            .map_err(|e| e.reason),
        Err(Reason::RelocationRequired)
    );
    // Stripped relocations or a fixed-base image still cannot move.
    exe.put16(0x96, 0x23);
    assert_eq!(
        app(&[exe.source("work.exe", moved)]).map(|_| ()),
        Err(Reason::RelocationRequired)
    );
    exe.put16(0x96, 0x22);
    exe.put16(OPTIONAL + 70, 0x120);
    assert_eq!(
        app(&[exe.source("work.exe", moved)]).map(|_| ()),
        Err(Reason::RelocationRequired)
    );
}
