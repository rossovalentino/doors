//! Fixed-base table of Door's original compatibility DLLs.
//!
//! The kernel maps an application's import closure from this table, the guest
//! adapter answers `GetModuleHandleW`/`GetModuleFileNameW` from it, and the
//! build places each DLL at its `base` (`/BASE:`). Bases are fixed so that the
//! read-only shared images need no per-process relocation; the table is the
//! single source of those addresses. A name here grants nothing by itself:
//! a module is present in a process only if its base appears in the trusted
//! bootstrap record.

/// Preferred and actual base of every Windows executable in this profile.
pub const EXECUTABLE_BASE: u64 = 0x1_4200_0000;
/// Address span reserved per fixed DLL base (at least the 16 MiB image cap).
pub const SLOT_BYTES: u64 = 0x0200_0000;
/// Spelling returned for system modules by `GetModuleFileNameW`.
pub const SYSTEM_DIRECTORY: &str = "C:\\Windows\\System32\\";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Provider {
    /// Lower-case file name as imported (case-insensitive match).
    pub name: &'static str,
    /// Authenticated initfs path of the original DLL image.
    pub path: &'static str,
    pub base: u64,
    /// Ordinary applications may import it. The P10 lifecycle fixture DLL is
    /// only mapped by the base fixture's explicit dynamic load.
    pub system: bool,
}

pub const NTDLL: usize = 0;
pub const KERNEL32: usize = 1;
pub const LIFECYCLE: usize = 2;
pub const MSVCRT: usize = 3;
pub const USER32: usize = 4;
pub const GDI32: usize = 5;
pub const COMDLG32: usize = 6;
pub const COMCTL32: usize = 7;
pub const WS2_32: usize = 8;
pub const IPHLPAPI: usize = 9;
pub const DNSAPI: usize = 10;
pub const WINHTTP: usize = 11;
pub const WININET: usize = 12;
pub const WINMM: usize = 13;
pub const ADVAPI32: usize = 14;
pub const DSOUND: usize = 15;
pub const OLE32: usize = 16;
pub const SHELL32: usize = 17;
pub const MSI: usize = 18;
pub const OLEAUT32: usize = 19;
pub const MMDEVAPI: usize = 20;
pub const TASKSCHD: usize = 21;
pub const WINTRUST: usize = 22;
pub const UCRTBASE: usize = 23;
/// P18: version.dll, forwarding to kernel32 as Windows' does to kernelbase.
pub const VERSION: usize = 24;
/// P18: bcrypt.dll (CNG random numbers and hashes).
pub const BCRYPT: usize = 25;
/// P18: secur32.dll (GetUserNameEx).
pub const SECUR32: usize = 26;
/// P18: amsi.dll (the Antimalware Scan Interface with no provider).
pub const AMSI: usize = 27;
/// P18: wldp.dll (Windows Lockdown Policy: no lockdown).
pub const WLDP: usize = 28;
/// P18: crypt32.dll (certificate stores and data protection).
pub const CRYPT32: usize = 29;
/// P18: dwrite.dll (DirectWrite fonts and glyph rendering).
pub const DWRITE: usize = 30;
/// P18: bcryptprimitives.dll (ProcessPrng, the process random generator).
pub const BCRYPTPRIMITIVES: usize = 31;
/// P18: uxtheme.dll (no visual styles; dark-mode preferences).
pub const UXTHEME: usize = 32;
/// P18: dwmapi.dll (composition always on).
pub const DWMAPI: usize = 33;
/// P18: wtsapi32.dll.
pub const WTSAPI32: usize = 34;
/// P18: shlwapi.dll.
pub const SHLWAPI: usize = 35;
/// P18: userenv.dll.
pub const USERENV: usize = 36;
/// P18: propsys.dll.
pub const PROPSYS: usize = 37;
/// P18: rpcrt4.dll.
pub const RPCRT4: usize = 38;
/// P18: oleacc.dll.
pub const OLEACC: usize = 39;
/// P18: uiautomationcore.dll.
pub const UIAUTOMATIONCORE: usize = 40;
/// P18: dbghelp.dll.
pub const DBGHELP: usize = 41;
/// P18: pdh.dll.
pub const PDH: usize = 42;
/// P18: tbs.dll.
pub const TBS: usize = 43;
/// P18: ncrypt.dll.
pub const NCRYPT: usize = 44;
/// P18: cryptui.dll.
pub const CRYPTUI: usize = 45;
/// P18: setupapi.dll.
pub const SETUPAPI: usize = 46;
/// P18: cfgmgr32.dll.
pub const CFGMGR32: usize = 47;
/// P18: hid.dll.
pub const HID: usize = 48;
/// P18: winusb.dll.
pub const WINUSB: usize = 49;
/// P18: mf.dll.
pub const MF: usize = 50;
/// P18: mfplat.dll.
pub const MFPLAT: usize = 51;
/// P18: mfreadwrite.dll.
pub const MFREADWRITE: usize = 52;
/// P18: wevtapi.dll.
pub const WEVTAPI: usize = 53;
/// P18: urlmon.dll.
pub const URLMON: usize = 54;
/// P18: dhcpcsvc.dll.
pub const DHCPCSVC: usize = 55;
/// P18: winspool.drv.
pub const WINSPOOL: usize = 56;
/// P18: bthprops.cpl.
pub const BTHPROPS: usize = 57;
/// P18: dxgi.dll.
pub const DXGI: usize = 58;
/// P18: d3d11.dll.
pub const D3D11: usize = 59;
/// P18: d3d12.dll.
pub const D3D12: usize = 60;
/// P18: dcomp.dll.
pub const DCOMP: usize = 61;
/// How many original DLLs the catalog holds.
pub const PROVIDER_COUNT: usize = 62;

pub const PROVIDERS: [Provider; PROVIDER_COUNT] = [
    provider("ntdll.dll", "windows/ntdll.dll", 0x1_8000_0000, true),
    provider("kernel32.dll", "windows/kernel32.dll", 0x1_8200_0000, true),
    provider(
        "lifecycle.dll",
        "windows/lifecycle.dll",
        0x1_8400_0000,
        false,
    ),
    provider("msvcrt.dll", "windows/msvcrt.dll", 0x1_8600_0000, true),
    provider("user32.dll", "windows/user32.dll", 0x1_8800_0000, true),
    provider("gdi32.dll", "windows/gdi32.dll", 0x1_8a00_0000, true),
    provider("comdlg32.dll", "windows/comdlg32.dll", 0x1_8c00_0000, true),
    provider("comctl32.dll", "windows/comctl32.dll", 0x1_8e00_0000, true),
    provider("ws2_32.dll", "windows/ws2_32.dll", 0x1_9000_0000, true),
    provider("iphlpapi.dll", "windows/iphlpapi.dll", 0x1_9200_0000, true),
    provider("dnsapi.dll", "windows/dnsapi.dll", 0x1_9400_0000, true),
    provider("winhttp.dll", "windows/winhttp.dll", 0x1_9600_0000, true),
    provider("wininet.dll", "windows/wininet.dll", 0x1_9800_0000, true),
    provider("winmm.dll", "windows/winmm.dll", 0x1_9a00_0000, true),
    provider("advapi32.dll", "windows/advapi32.dll", 0x1_9c00_0000, true),
    provider("dsound.dll", "windows/dsound.dll", 0x1_9e00_0000, true),
    provider("ole32.dll", "windows/ole32.dll", 0x1_a000_0000, true),
    provider("shell32.dll", "windows/shell32.dll", 0x1_a200_0000, true),
    provider("msi.dll", "windows/msi.dll", 0x1_a400_0000, true),
    provider("oleaut32.dll", "windows/oleaut32.dll", 0x1_a600_0000, true),
    provider("mmdevapi.dll", "windows/mmdevapi.dll", 0x1_a800_0000, true),
    provider("taskschd.dll", "windows/taskschd.dll", 0x1_aa00_0000, true),
    provider("wintrust.dll", "windows/wintrust.dll", 0x1_ac00_0000, true),
    provider("ucrtbase.dll", "windows/ucrtbase.dll", 0x1_ae00_0000, true),
    provider("version.dll", "windows/version.dll", 0x1_b000_0000, true),
    provider("bcrypt.dll", "windows/bcrypt.dll", 0x1_b200_0000, true),
    provider("secur32.dll", "windows/secur32.dll", 0x1_b400_0000, true),
    provider("amsi.dll", "windows/amsi.dll", 0x1_b600_0000, true),
    provider("wldp.dll", "windows/wldp.dll", 0x1_b800_0000, true),
    provider("crypt32.dll", "windows/crypt32.dll", 0x1_ba00_0000, true),
    provider("dwrite.dll", "windows/dwrite.dll", 0x1_bc00_0000, true),
    provider(
        "bcryptprimitives.dll",
        "windows/bcryptprimitives.dll",
        0x1_be00_0000,
        true,
    ),
    provider("uxtheme.dll", "windows/uxtheme.dll", 0x1_c000_0000, true),
    provider("dwmapi.dll", "windows/dwmapi.dll", 0x1_c200_0000, true),
    provider("wtsapi32.dll", "windows/wtsapi32.dll", 0x1_c400_0000, true),
    provider("shlwapi.dll", "windows/shlwapi.dll", 0x1_c600_0000, true),
    provider("userenv.dll", "windows/userenv.dll", 0x1_c800_0000, true),
    provider("propsys.dll", "windows/propsys.dll", 0x1_ca00_0000, true),
    provider("rpcrt4.dll", "windows/rpcrt4.dll", 0x1_cc00_0000, true),
    provider("oleacc.dll", "windows/oleacc.dll", 0x1_ce00_0000, true),
    provider(
        "uiautomationcore.dll",
        "windows/uiautomationcore.dll",
        0x1_d000_0000,
        true,
    ),
    provider("dbghelp.dll", "windows/dbghelp.dll", 0x1_d200_0000, true),
    provider("pdh.dll", "windows/pdh.dll", 0x1_d400_0000, true),
    provider("tbs.dll", "windows/tbs.dll", 0x1_d600_0000, true),
    provider("ncrypt.dll", "windows/ncrypt.dll", 0x1_d800_0000, true),
    provider("cryptui.dll", "windows/cryptui.dll", 0x1_da00_0000, true),
    provider("setupapi.dll", "windows/setupapi.dll", 0x1_dc00_0000, true),
    provider("cfgmgr32.dll", "windows/cfgmgr32.dll", 0x1_de00_0000, true),
    provider("hid.dll", "windows/hid.dll", 0x1_e000_0000, true),
    provider("winusb.dll", "windows/winusb.dll", 0x1_e200_0000, true),
    provider("mf.dll", "windows/mf.dll", 0x1_e400_0000, true),
    provider("mfplat.dll", "windows/mfplat.dll", 0x1_e600_0000, true),
    provider(
        "mfreadwrite.dll",
        "windows/mfreadwrite.dll",
        0x1_e800_0000,
        true,
    ),
    provider("wevtapi.dll", "windows/wevtapi.dll", 0x1_ea00_0000, true),
    provider("urlmon.dll", "windows/urlmon.dll", 0x1_ec00_0000, true),
    provider("dhcpcsvc.dll", "windows/dhcpcsvc.dll", 0x1_ee00_0000, true),
    provider("winspool.drv", "windows/winspool.drv", 0x1_f000_0000, true),
    provider("bthprops.cpl", "windows/bthprops.cpl", 0x1_f200_0000, true),
    provider("dxgi.dll", "windows/dxgi.dll", 0x1_f400_0000, true),
    provider("d3d11.dll", "windows/d3d11.dll", 0x1_f600_0000, true),
    provider("d3d12.dll", "windows/d3d12.dll", 0x1_f800_0000, true),
    provider("dcomp.dll", "windows/dcomp.dll", 0x1_fa00_0000, true),
];

/// DLLs an application loads that are not system DLLs (LoadLibrary of its
/// own or an installer's DLL): at most this many per process, each in its
/// own slot after the last system DLL, relocated there.
pub const PRIVATE_MODULES: usize = 4;
/// The names those DLLs carry in the process's module plan.
pub const PRIVATE_NAMES: [&str; PRIVATE_MODULES] = [
    "door-private-0.dll",
    "door-private-1.dll",
    "door-private-2.dll",
    "door-private-3.dll",
];
/// Base of private slot `index`.
pub const fn private_base(index: usize) -> u64 {
    PRIVATE_BASE + index as u64 * SLOT_BYTES
}
/// After room for 128 catalog DLLs (0x1_8000_0000 + 128 * 32 MiB), so
/// adding a system DLL never moves the private slots.
pub const PRIVATE_BASE: u64 = 0x2_8000_0000;
/// The private slot of a module base, if it is one.
pub fn private_slot(base: u64) -> Option<usize> {
    (0..PRIVATE_MODULES).find(|&index| private_base(index) == base)
}

/// x86 (P17): the base of a 32-bit program's executable when it can move
/// (its preferred base otherwise, see `executable32_base`).
pub const EXECUTABLE32_BASE: u64 = 0x0040_0000;
/// Span reserved per 32-bit private DLL slot (the 16 MiB image cap: an
/// application's own DLL may be that large).
pub const SLOT32_BYTES: u64 = 0x0100_0000;
/// Span reserved per 32-bit system DLL (P18: 8 MiB, so the catalog fits
/// below the large-executable range; the build checks each 32-bit system
/// DLL's image fits, the largest being about 2 MiB).
pub const PROVIDER32_BYTES: u64 = 0x0080_0000;
/// First 32-bit DLL slot; the executable's range lies below it.
pub const PROVIDERS32_BASE: u64 = 0x0200_0000;
/// The folder the 32-bit system DLLs come from (`GetSystemWow64DirectoryW`).
/// A 32-bit program still names them under System32 (`GetModuleFileNameW`,
/// `GetSystemDirectoryW`), as file system redirection shows them on Windows
/// (measured: `x86probe.exe raw`).
pub const SYSTEM_DIRECTORY32: &str = "C:\\Windows\\SysWOW64\\";
/// The 32-bit builds of the providers, indexed like `PROVIDERS`: the same
/// names, initfs `windows32/<name>`, slots from `PROVIDERS32_BASE`.
pub const PROVIDERS32: [Provider; PROVIDER_COUNT] = {
    let mut table = [provider("", "", 0, false); PROVIDER_COUNT];
    let mut index = 0;
    while index < PROVIDER_COUNT {
        table[index] = provider(
            PROVIDERS[index].name,
            PATHS32[index],
            PROVIDERS32_BASE + index as u64 * PROVIDER32_BYTES,
            PROVIDERS[index].system,
        );
        index += 1;
    }
    table
};
const PATHS32: [&str; PROVIDER_COUNT] = [
    "windows32/ntdll.dll",
    "windows32/kernel32.dll",
    "windows32/lifecycle.dll",
    "windows32/msvcrt.dll",
    "windows32/user32.dll",
    "windows32/gdi32.dll",
    "windows32/comdlg32.dll",
    "windows32/comctl32.dll",
    "windows32/ws2_32.dll",
    "windows32/iphlpapi.dll",
    "windows32/dnsapi.dll",
    "windows32/winhttp.dll",
    "windows32/wininet.dll",
    "windows32/winmm.dll",
    "windows32/advapi32.dll",
    "windows32/dsound.dll",
    "windows32/ole32.dll",
    "windows32/shell32.dll",
    "windows32/msi.dll",
    "windows32/oleaut32.dll",
    "windows32/mmdevapi.dll",
    "windows32/taskschd.dll",
    "windows32/wintrust.dll",
    "windows32/ucrtbase.dll",
    "windows32/version.dll",
    "windows32/bcrypt.dll",
    "windows32/secur32.dll",
    "windows32/amsi.dll",
    "windows32/wldp.dll",
    "windows32/crypt32.dll",
    "windows32/dwrite.dll",
    "windows32/bcryptprimitives.dll",
    "windows32/uxtheme.dll",
    "windows32/dwmapi.dll",
    "windows32/wtsapi32.dll",
    "windows32/shlwapi.dll",
    "windows32/userenv.dll",
    "windows32/propsys.dll",
    "windows32/rpcrt4.dll",
    "windows32/oleacc.dll",
    "windows32/uiautomationcore.dll",
    "windows32/dbghelp.dll",
    "windows32/pdh.dll",
    "windows32/tbs.dll",
    "windows32/ncrypt.dll",
    "windows32/cryptui.dll",
    "windows32/setupapi.dll",
    "windows32/cfgmgr32.dll",
    "windows32/hid.dll",
    "windows32/winusb.dll",
    "windows32/mf.dll",
    "windows32/mfplat.dll",
    "windows32/mfreadwrite.dll",
    "windows32/wevtapi.dll",
    "windows32/urlmon.dll",
    "windows32/dhcpcsvc.dll",
    "windows32/winspool.drv",
    "windows32/bthprops.cpl",
    "windows32/dxgi.dll",
    "windows32/d3d11.dll",
    "windows32/d3d12.dll",
    "windows32/dcomp.dll",
];
/// Where a 32-bit executable is mapped: its preferred base when the whole
/// image fits below the first DLL slot or in the large-image range after
/// the private slots (an image without relocations can only go there);
/// else `EXECUTABLE32_BASE` when it fits there, or `LARGE_EXECUTABLE32_BASE`.
pub const fn executable32_base(preferred: u64, image_bytes: u64) -> u64 {
    let aligned = preferred >= 0x1_0000 && preferred.is_multiple_of(0x1_0000);
    let low = preferred + image_bytes <= PROVIDERS32_BASE;
    let large =
        preferred >= LARGE_EXECUTABLE32_BASE && preferred + image_bytes <= LARGE_EXECUTABLE32_END;
    if aligned && (low || large) {
        preferred
    } else if EXECUTABLE32_BASE + image_bytes <= PROVIDERS32_BASE {
        EXECUTABLE32_BASE
    } else {
        LARGE_EXECUTABLE32_BASE
    }
}
/// Executables too large for the space below the DLL slots go after the
/// private slots (with room for more catalog DLLs), below the process heap
/// (0x5000_0000).
pub const LARGE_EXECUTABLE32_BASE: u64 = 0x2800_0000;
pub const LARGE_EXECUTABLE32_END: u64 = 0x5000_0000;
const _: () =
    assert!(PRIVATE32_BASE + PRIVATE_MODULES as u64 * SLOT32_BYTES <= LARGE_EXECUTABLE32_BASE);
/// Base of 32-bit private slot `index`, after the 32-bit providers.
pub const fn private32_base(index: usize) -> u64 {
    PRIVATE32_BASE + index as u64 * SLOT32_BYTES
}
pub const PRIVATE32_BASE: u64 = PROVIDERS32_BASE + PROVIDERS32.len() as u64 * PROVIDER32_BYTES;

const fn provider(name: &'static str, path: &'static str, base: u64, system: bool) -> Provider {
    Provider {
        name,
        path,
        base,
        system,
    }
}

const _: () = {
    let mut index = 0;
    while index < PROVIDERS.len() {
        let base = PROVIDERS[index].base;
        assert!(base.is_multiple_of(0x1_0000));
        assert!(base >= EXECUTABLE_BASE + SLOT_BYTES);
        if index > 0 {
            assert!(base >= PROVIDERS[index - 1].base + SLOT_BYTES);
        }
        index += 1;
    }
    assert!(PROVIDERS[PROVIDERS.len() - 1].base + SLOT_BYTES <= 0x8000_0000_0000);
    assert!(PRIVATE_BASE >= PROVIDERS[PROVIDERS.len() - 1].base + SLOT_BYTES);
    assert!(PRIVATE_BASE.is_multiple_of(0x1_0000));
    assert!(private_base(PRIVATE_MODULES) <= 0x8000_0000_0000);
    // The 32-bit slots lie between the executable and the process heap
    // (0x5000_0000), each within the 16 MiB image cap.
    assert!(SLOT32_BYTES >= 16 * 1024 * 1024);
    assert!(PROVIDER32_BYTES >= 8 * 1024 * 1024 && PROVIDER32_BYTES.is_multiple_of(0x1_0000));
    assert!(EXECUTABLE32_BASE + 0x0100_0000 <= PROVIDERS32_BASE);
    assert!(private32_base(PRIVATE_MODULES) <= 0x5000_0000);
};
/// `by_name` over the 32-bit catalog.
pub fn by_name32(name: &str) -> Option<(usize, &'static Provider)> {
    by_name(name).map(|(index, _)| (index, &PROVIDERS32[index]))
}
const _: () = {
    assert!(PROVIDERS32.len() == PROVIDERS.len());
};

/// Case-insensitive lookup of an imported or requested module file name.
/// A bare stem (no extension) matches as `LoadLibrary`/`GetModuleHandle` do.
pub fn by_name(name: &str) -> Option<(usize, &'static Provider)> {
    PROVIDERS.iter().enumerate().find(|(_, provider)| {
        provider.name.eq_ignore_ascii_case(name)
            || (!name.contains('.')
                && provider
                    .name
                    .strip_suffix(".dll")
                    .is_some_and(|stem| stem.eq_ignore_ascii_case(name)))
    })
}

pub fn by_base(base: u64) -> Option<(usize, &'static Provider)> {
    PROVIDERS
        .iter()
        .enumerate()
        .find(|(_, provider)| provider.base == base)
}

/// The catalog as Doors' own DLLs see it: each build its own
/// architecture's (P17: the x86 DLLs see the 32-bit slots). Both name the
/// system modules' folder System32, as WOW64's redirection presents it.
/// The kernel uses the tables directly.
pub mod own {
    use super::Provider;
    #[cfg(target_arch = "x86")]
    pub const PROVIDERS: &[Provider; super::PROVIDER_COUNT] = &super::PROVIDERS32;
    #[cfg(not(target_arch = "x86"))]
    pub const PROVIDERS: &[Provider; super::PROVIDER_COUNT] = &super::PROVIDERS;
    pub const SYSTEM_DIRECTORY: &str = super::SYSTEM_DIRECTORY;
    #[cfg(target_arch = "x86")]
    pub const EXECUTABLE_BASE: u64 = super::EXECUTABLE32_BASE;
    #[cfg(not(target_arch = "x86"))]
    pub const EXECUTABLE_BASE: u64 = super::EXECUTABLE_BASE;
    pub const fn private_base(index: usize) -> u64 {
        #[cfg(target_arch = "x86")]
        {
            super::private32_base(index)
        }
        #[cfg(not(target_arch = "x86"))]
        {
            super::private_base(index)
        }
    }
    pub fn private_slot(base: u64) -> Option<usize> {
        (0..super::PRIVATE_MODULES).find(|&index| private_base(index) == base)
    }
    pub fn by_name(name: &str) -> Option<(usize, &'static Provider)> {
        super::by_name(name).map(|(index, _)| (index, &PROVIDERS[index]))
    }
    pub fn by_base(base: u64) -> Option<(usize, &'static Provider)> {
        PROVIDERS
            .iter()
            .enumerate()
            .find(|(_, provider)| provider.base == base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_bases_are_unique_aligned_and_resolve_by_spelling() {
        assert_eq!(by_name("KERNEL32.DLL").unwrap().0, KERNEL32);
        assert_eq!(by_name("User32").unwrap().0, USER32);
        assert_eq!(by_name("user32.").map(|p| p.0), None);
        assert_eq!(by_name("gdi32.dll.dll"), None);
        assert_eq!(by_name("..\\gdi32.dll"), None);
        assert_eq!(by_base(0x1_8a00_0000).unwrap().1.name, "gdi32.dll");
        // Private DLL slots follow the system DLLs and name no provider.
        assert_eq!(private_slot(private_base(2)), Some(2));
        assert_eq!(private_slot(PROVIDERS[MSI].base), None);
        assert!(by_base(private_base(0)).is_none());
        assert!(PRIVATE_NAMES.iter().all(|n| by_name(n).is_none()));
        assert_eq!(by_base(0x1_8a00_1000), None);
        assert!(!PROVIDERS[LIFECYCLE].system);
        for (index, provider) in PROVIDERS.iter().enumerate() {
            assert_eq!(by_name(provider.name).unwrap().0, index);
            assert_eq!(provider.path, &*format!("windows/{}", provider.name));
        }
    }
    extern crate std;
    use std::format;
}
