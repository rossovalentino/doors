use door_pe_loader::api_set_host;

#[test]
fn contracts_resolve_by_level_and_every_minor_version() {
    for name in [
        "api-ms-win-core-memory-l1-1-0.dll",
        "API-MS-WIN-CORE-MEMORY-L1-1-6.DLL",
        "api-ms-win-core-memory-l1-1-8",
        "api-ms-win-core-synch-l1-2-0.dll",
        "kernelbase.dll",
    ] {
        assert_eq!(api_set_host(name), Some("kernel32.dll"), "{name}");
    }
    assert_eq!(
        api_set_host("api-ms-win-crt-runtime-l1-1-0.dll"),
        Some("ucrtbase.dll")
    );
    assert_eq!(
        api_set_host("api-ms-win-core-rtlsupport-l1-2-0.dll"),
        Some("ntdll.dll")
    );
    assert_eq!(
        api_set_host("api-ms-win-core-registry-l1-1-0.dll"),
        Some("advapi32.dll")
    );
    // Unknown levels and contracts, and names that are not contracts.
    for name in [
        "api-ms-win-core-synch-l9-9-9.dll",
        "api-ms-win-core-nothing-l1-1-0.dll",
        "api-ms-win-core-memory-l1-1-x.dll",
        "api-ms-win-core-memory-l1-1-.dll",
        "kernel32.dll",
        "",
        ".dll",
    ] {
        assert_eq!(api_set_host(name), None, "{name}");
    }
}
