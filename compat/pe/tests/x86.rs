//! P17: PE32 (x86) bundles under `LoadPolicy::Application32`, with real
//! MSVC x86 images (`data/x86`, see its build.md): HIGHLOW relocations,
//! imports by name and by ordinal, a TLS directory with a callback, a 32-bit
//! load configuration with a /GS cookie and a SafeSEH table.
use door_pe_loader::{
    BundlePlan, DEFAULT_SECURITY_COOKIE_X86, FixupKind, ImageBase, ImageError, LoadPolicy, Machine,
    ModuleSource, Reason, Rva, Symbol,
};

const APP: &[u8] = include_bytes!("data/x86/x86app.exe");
const DLL: &[u8] = include_bytes!("data/x86/x86dll.dll");
/// Bases other than the preferred 0x400000 and 0x10000000, so every
/// absolute address moves.
const APP_BASE: u64 = 0x0050_0000;
const DLL_BASE: u64 = 0x7700_0000;

fn sources(app: u64, dll: u64) -> [ModuleSource<'static>; 2] {
    [
        ModuleSource {
            name: "x86app.exe",
            bytes: APP,
            base: ImageBase(app),
        },
        ModuleSource {
            name: "x86dll.dll",
            bytes: DLL,
            base: ImageBase(dll),
        },
    ]
}

#[test]
fn x86_bundles_relocate_bind_and_describe_tls_and_safe_seh() {
    let plan =
        BundlePlan::with_policy(&sources(APP_BASE, DLL_BASE), LoadPolicy::Application32).unwrap();
    for index in 0..2 {
        let module = plan.module(index).unwrap();
        assert_eq!(module.machine(), Machine::X86);
        assert_eq!(module.pointer_bytes(), 4);
    }
    assert_eq!(plan.entry().0, APP_BASE + 0x1020);
    assert_eq!(plan.subsystem(), 3);
    let add = plan.resolve_export(1, Symbol::Name("DoorAdd")).unwrap();
    let twice = plan.resolve_export(1, Symbol::Ordinal(2)).unwrap();
    assert_eq!((add.0, twice.0), (DLL_BASE + 0x1006, DLL_BASE + 0x1015));
    let mut relocations = [0usize; 2];
    let mut imports = Vec::new();
    let mut cookies = Vec::new();
    plan.visit_fixups_with_cookie(0x1234_5678_9abc_def0, |fixup| {
        assert_eq!(fixup.width, 4);
        assert!(fixup.value <= u64::from(u32::MAX));
        let (bytes, width) = fixup.bytes();
        assert_eq!(width, 4);
        assert_eq!(
            u64::from(u32::from_le_bytes(bytes[..4].try_into().unwrap())),
            fixup.value
        );
        match fixup.kind {
            FixupKind::HighLow => {
                relocations[fixup.module] += 1;
                let (base, size) = match fixup.module {
                    0 => (APP_BASE, 0x5000),
                    _ => (DLL_BASE, 0x5000),
                };
                assert!(
                    fixup.value >= base && fixup.value < base + size,
                    "{fixup:?}"
                );
            }
            FixupKind::Import => imports.push(fixup.value),
            FixupKind::SecurityCookie => cookies.push((fixup.module, fixup.value)),
            other => panic!("{other:?}"),
        }
        Ok::<(), ()>(())
    })
    .unwrap();
    assert_eq!(relocations, [17, 3]);
    imports.sort();
    assert_eq!(imports, [add.0, twice.0]);
    // The executable's cookie slot held the x86 compiler default: rewritten
    // to a 32-bit value that is neither zero nor that default.
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].0, 0);
    assert!(cookies[0].1 != 0 && cookies[0].1 != DEFAULT_SECURITY_COOKIE_X86);
    let app = plan.module(0).unwrap();
    let tls = app.tls().unwrap().unwrap();
    assert_eq!(tls.template, &0x1234u32.to_le_bytes());
    assert_eq!(tls.template_rva, Rva(0x22ac));
    assert_eq!(tls.index, Rva(0x3008));
    assert_eq!(tls.callback_count, 1);
    assert_eq!(plan.tls_slot(0), Some(0));
    let config = app.load_config().unwrap().unwrap();
    assert_eq!(config.security_cookie, Some(Rva(0x3000)));
    assert_eq!(config.safe_seh, Some((Rva(0x2138), 1)));
}

#[test]
fn x86_images_need_the_x86_policy_and_low_bases() {
    // The x64 policies refuse PE32 images, and the x86 one refuses PE32+.
    let error =
        BundlePlan::with_policy(&sources(APP_BASE, DLL_BASE), LoadPolicy::Application).unwrap_err();
    assert_eq!(error.reason, Reason::Image(ImageError::Unsupported));
    let x64 = [ModuleSource {
        name: "x86app.exe",
        bytes: include_bytes!("../../../crates/authenticode/tests/data/unsigned.exe"),
        base: ImageBase(0x1_4000_0000),
    }];
    let error = BundlePlan::with_policy(&x64, LoadPolicy::Application32).unwrap_err();
    assert_eq!(error.reason, Reason::Image(ImageError::Unsupported));
    // An x86 image must lie wholly below 2 GB (its addresses are widened
    // with sign extension).
    for base in [0x1_0000_0000, 0x8000_0000, 0xffff_0000] {
        let error = BundlePlan::with_policy(&sources(APP_BASE, base), LoadPolicy::Application32)
            .unwrap_err();
        assert_eq!((error.module, error.reason), (1, Reason::InvalidAddress));
    }
    // The last 64 KB slot below 2 GB still holds the 20 KB image.
    assert!(
        BundlePlan::with_policy(&sources(APP_BASE, 0x7fff_0000), LoadPolicy::Application32).is_ok()
    );
}

#[test]
fn mutated_x86_images_never_panic() {
    for index in (0..APP.len()).step_by(3) {
        for value in [0u8, 0xff] {
            let mut bytes = APP.to_vec();
            bytes[index] = value;
            let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
            let sources = [
                ModuleSource {
                    name: "x86app.exe",
                    bytes: leaked,
                    base: ImageBase(APP_BASE),
                },
                ModuleSource {
                    name: "x86dll.dll",
                    bytes: DLL,
                    base: ImageBase(DLL_BASE),
                },
            ];
            if let Ok(plan) = BundlePlan::with_policy(&sources, LoadPolicy::Application32) {
                let _ = plan.visit_fixups(|_| Ok::<(), ()>(()));
                let _ = plan.module(0).unwrap().tls();
            }
        }
    }
}
