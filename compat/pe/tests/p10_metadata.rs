mod support;
use door_pe_loader::{BundlePlan, Feature, LoadPolicy, Reason};
use support::*;

#[test]
fn deferred_images_cannot_satisfy_startup_imports_or_forwarders() {
    let mut exe = Image::new(false);
    let mut pinned = Image::new(true);
    let mut app = Image::new(true);
    app.exports(&[("Run", 0x1000)]);
    let validate = |exe: &Image, pinned: &Image| {
        BundlePlan::with_policy(
            &[
                exe.source("app.exe", EXE_BASE),
                pinned.source("pinned.dll", DLL_BASE),
                app.source("late.dll", DLL_BASE + 0x10000),
            ],
            LoadPolicy::DllCallbacks,
        )
        .unwrap()
        .validate_deferred_module(2)
    };
    assert_eq!(validate(&exe, &pinned), Ok(()));
    exe.imports("late.dll", &["Run"]);
    assert_eq!(
        validate(&exe, &pinned).unwrap_err().reason,
        Reason::DeferredDependency
    );
    exe = Image::new(false);
    pinned.exports(&[("Run", 0x3400)]);
    pinned.string(0x3400, "late.Run");
    assert_eq!(
        validate(&exe, &pinned).unwrap_err().reason,
        Reason::DeferredDependency
    );
}
fn tls_image() -> Image {
    let mut dll = Image::new(true);
    dll.directory(9, 0x2800, 40);
    dll.rva64(0x2800, DLL_BASE + u64::from(DATA + 32));
    dll.rva64(0x2808, DLL_BASE + u64::from(DATA + 40));
    dll.rva64(0x2810, DLL_BASE + u64::from(DATA + 16));
    dll.rva64(0x2818, DLL_BASE + 0x2900);
    dll.rva32(0x2820, 16);
    dll.rva64(0x2900, DLL_BASE + 0x1000);
    dll.rva64(0x2908, 0);
    dll.rva64(DATA + 32, 0x1122334455667788);
    dll
}
#[test]
fn compiler_tls_is_explicit_policy_with_bounded_owned_template_and_callbacks() {
    let exe = Image::new(false);
    let dll = tls_image();
    let inputs = [
        exe.source("app.exe", EXE_BASE),
        dll.source("tls.dll", DLL_BASE),
    ];
    assert!(matches!(
        BundlePlan::new(&inputs).err().unwrap().reason,
        Reason::Unsupported(Feature::Tls)
    ));
    let plan = BundlePlan::with_policy(&inputs, LoadPolicy::DllCallbacks).unwrap();
    let tls = plan.module(1).unwrap().tls().unwrap().unwrap();
    assert_eq!(tls.template, &0x1122334455667788u64.to_le_bytes());
    assert_eq!(tls.zero_bytes, 16);
    assert_eq!(tls.index.0, DATA + 16);
    assert_eq!(tls.callback_count, 1);
    assert_eq!(tls.callbacks[0].0, 0x1000);
}
#[test]
fn tls_rejects_foreign_pointers_readonly_index_noncode_callbacks_and_exhaustion() {
    let exe = Image::new(false);
    for (rva, value) in [
        (0x2800, 1),
        (0x2808, DLL_BASE + u64::from(DATA + 31)),
        (0x2810, DLL_BASE + 0x2000),
        (0x2900, DLL_BASE + u64::from(DATA)),
        (0x2808, DLL_BASE + u64::from(DATA + 400)),
    ] {
        let mut dll = tls_image();
        dll.rva64(rva, value);
        let inputs = [
            exe.source("app.exe", EXE_BASE),
            dll.source("tls.dll", DLL_BASE),
        ];
        assert!(
            BundlePlan::with_policy(&inputs, LoadPolicy::DllCallbacks).is_err(),
            "field {rva:x}"
        );
    }
    let mut dll = tls_image();
    for i in 0..9 {
        dll.rva64(0x2900 + i * 8, DLL_BASE + 0x1000);
    }
    assert!(
        BundlePlan::with_policy(
            &[
                exe.source("app.exe", EXE_BASE),
                dll.source("tls.dll", DLL_BASE)
            ],
            LoadPolicy::DllCallbacks
        )
        .is_err()
    );
}
#[test]
fn api_contract_redirects_are_p10_only_and_unknown_contracts_fail() {
    let mut exe = Image::new(false);
    exe.imports("API-MS-WIN-CORE-SYNCH-L1-2-0.DLL", &["Sleep"]);
    let mut dll = Image::new(true);
    dll.exports(&[("Sleep", 0x1000)]);
    let inputs = [
        exe.source("app.exe", EXE_BASE),
        dll.source("kernel32.dll", DLL_BASE),
    ];
    assert_eq!(
        BundlePlan::new(&inputs).err().unwrap().reason,
        Reason::MissingModule
    );
    BundlePlan::with_policy(&inputs, LoadPolicy::DllCallbacks).unwrap();
    exe.imports("api-ms-win-core-synch-l9-9-9.dll", &["Sleep"]);
    assert_eq!(
        BundlePlan::with_policy(
            &[
                exe.source("app.exe", EXE_BASE),
                dll.source("kernel32.dll", DLL_BASE)
            ],
            LoadPolicy::DllCallbacks
        )
        .err()
        .unwrap()
        .reason,
        Reason::MissingModule
    );
}
