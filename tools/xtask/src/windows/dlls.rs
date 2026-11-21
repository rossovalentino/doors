//! Original DLLs use the installed Windows target compiler without system DLL code.
use crate::{
    artifact,
    evidence::{Run, file_hash, relative},
};
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};

/// Original DLLs of every P09+ image, in build (dependency) order: each links
/// against the import libraries of the DLLs before it in the same target.
/// The GUI DLLs (P13) and the P15 network and multimedia DLLs are staged at
/// `windows/<name>.dll` like the base pair.
pub const NAMES: [&str; 60] = [
    "ntdll",
    "kernel32",
    "ucrtbase",
    "gdi32",
    "user32",
    "comctl32",
    "comdlg32",
    "ws2_32",
    "iphlpapi",
    "dnsapi",
    "advapi32",
    "winhttp",
    "wininet",
    "winmm",
    "dsound",
    "ole32",
    "shell32",
    "msi",
    "oleaut32",
    "mmdevapi",
    "taskschd",
    "wintrust",
    "version",
    "bcrypt",
    "secur32",
    "amsi",
    "wldp",
    "crypt32",
    "dwrite",
    "bcryptprimitives",
    "uxtheme",
    "dwmapi",
    "wtsapi32",
    "shlwapi",
    "userenv",
    "propsys",
    "rpcrt4",
    "oleacc",
    "uiautomationcore",
    "dbghelp",
    "pdh",
    "tbs",
    "ncrypt",
    "cryptui",
    "setupapi",
    "cfgmgr32",
    "hid",
    "winusb",
    "mf",
    "mfplat",
    "mfreadwrite",
    "wevtapi",
    "urlmon",
    "dhcpcsvc",
    "winspool",
    "bthprops",
    "dxgi",
    "d3d11",
    "d3d12",
    "dcomp",
];
/// P17: the original DLLs also built for 32-bit programs
/// (`i686-pc-windows-msvc`), staged at `windows32/<name>.dll` (WOW64's
/// SysWOW64), at the 32-bit catalog's bases. The same sources as the x64
/// builds.
pub const NAMES32: [&str; 61] = [
    "ntdll",
    "kernel32",
    "ucrtbase",
    "msvcrt",
    "gdi32",
    "user32",
    "comctl32",
    "comdlg32",
    "ws2_32",
    "iphlpapi",
    "dnsapi",
    "advapi32",
    "winhttp",
    "wininet",
    "winmm",
    "dsound",
    "ole32",
    "shell32",
    "msi",
    "oleaut32",
    "mmdevapi",
    "taskschd",
    "wintrust",
    "version",
    "bcrypt",
    "secur32",
    "amsi",
    "wldp",
    "crypt32",
    "dwrite",
    "bcryptprimitives",
    "uxtheme",
    "dwmapi",
    "wtsapi32",
    "shlwapi",
    "userenv",
    "propsys",
    "rpcrt4",
    "oleacc",
    "uiautomationcore",
    "dbghelp",
    "pdh",
    "tbs",
    "ncrypt",
    "cryptui",
    "setupapi",
    "cfgmgr32",
    "hid",
    "winusb",
    "mf",
    "mfplat",
    "mfreadwrite",
    "wevtapi",
    "urlmon",
    "dhcpcsvc",
    "winspool",
    "bthprops",
    "dxgi",
    "d3d11",
    "d3d12",
    "dcomp",
];
/// The file an original DLL is installed as: `<name>.dll`, except the
/// print spooler (`winspool.drv`) and the Bluetooth control panel
/// (`bthprops.cpl`), which Windows names so.
pub fn file_name(name: &str) -> String {
    match name {
        "winspool" => String::from("winspool.drv"),
        "bthprops" => String::from("bthprops.cpl"),
        _ => format!("{name}.dll"),
    }
}
/// The catalog index of an original DLL (`door_windows_abi::modules`).
fn catalog_index(name: &str) -> Result<u64, String> {
    const ORDER: [&str; 62] = [
        "ntdll",
        "kernel32",
        "lifecycle",
        "msvcrt",
        "user32",
        "gdi32",
        "comdlg32",
        "comctl32",
        "ws2_32",
        "iphlpapi",
        "dnsapi",
        "winhttp",
        "wininet",
        "winmm",
        "advapi32",
        "dsound",
        "ole32",
        "shell32",
        "msi",
        "oleaut32",
        "mmdevapi",
        "taskschd",
        "wintrust",
        "ucrtbase",
        "version",
        "bcrypt",
        "secur32",
        "amsi",
        "wldp",
        "crypt32",
        "dwrite",
        "bcryptprimitives",
        "uxtheme",
        "dwmapi",
        "wtsapi32",
        "shlwapi",
        "userenv",
        "propsys",
        "rpcrt4",
        "oleacc",
        "uiautomationcore",
        "dbghelp",
        "pdh",
        "tbs",
        "ncrypt",
        "cryptui",
        "setupapi",
        "cfgmgr32",
        "hid",
        "winusb",
        "mf",
        "mfplat",
        "mfreadwrite",
        "wevtapi",
        "urlmon",
        "dhcpcsvc",
        "winspool",
        "bthprops",
        "dxgi",
        "d3d11",
        "d3d12",
        "dcomp",
    ];
    let index = ORDER
        .iter()
        .position(|n| *n == name)
        .ok_or("unknown original DLL")?;
    Ok(index as u64)
}
/// The 32-bit catalog base of an original DLL: slot `index` of
/// `PROVIDERS32` (from 0x0200_0000, 8 MiB each, in the catalog's order).
fn base32(name: &str) -> Result<u64, String> {
    Ok(0x0200_0000 + catalog_index(name)? * PROVIDER32_BYTES)
}
/// The x64 catalog base: slot `index` of `PROVIDERS` (from 0x1_8000_0000,
/// 32 MiB each).
fn base64(name: &str) -> Result<u64, String> {
    Ok(0x1_8000_0000 + catalog_index(name)? * 0x0200_0000)
}
/// The span of a 32-bit system DLL slot.
const PROVIDER32_BYTES: u64 = 0x0080_0000;
/// Original Windows programs (Rust, no C runtime), staged as seeds and
/// installed into C:\Windows\System32: (name, imports).
pub const PROGRAMS: [(&str, &[&str]); 2] = [
    ("msiexec", &["kernel32.dll", "msi.dll"]),
    ("imagehost", &["kernel32.dll"]),
];
/// P13 GUI compatibility DLLs (P13-B crates), staged at `windows/<name>.dll`.
pub const GUI_NAMES: [&str; 4] = ["gdi32", "user32", "comctl32", "comdlg32"];
/// The exact import graph of each original DLL (sorted lower-case module
/// names): only Door's own lower modules, never a host DLL.
pub fn allowed_imports(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "ntdll" => &[],
        "kernel32" | "msvcrt" => &["ntdll.dll"],
        // P18 M0d: the Universal CRT; console, file and environment calls
        // through kernel32, the heap and threads through ntdll.
        "ucrtbase" => &["kernel32.dll", "ntdll.dll"],
        "gdi32" => &["ntdll.dll"],
        "user32" => &["gdi32.dll", "ntdll.dll"],
        "comctl32" => &["gdi32.dll", "ntdll.dll", "user32.dll"],
        "comdlg32" => &["ntdll.dll", "user32.dll"],
        // P15: Winsock over door.network.
        "ws2_32" => &["ntdll.dll"],
        // P15: IP Helper over door.network.
        "iphlpapi" => &["ntdll.dll"],
        // P15: DNS client over door.network sockets.
        "dnsapi" => &["ntdll.dll"],
        // P15: WinHTTP over door.network and door-tls; the user's proxy
        // settings (Internet Settings) through advapi32.
        "winhttp" => &["advapi32.dll", "ntdll.dll"],
        // P15: WinINet over the same transport; the proxy settings through
        // advapi32, the URL cache's files through kernel32.
        "wininet" => &["advapi32.dll", "kernel32.dll", "ntdll.dll"],
        // P15: Windows Multimedia over door.audio.
        "winmm" => &["ntdll.dll"],
        // P16: registry, tokens and SIDs over door.storage and the identity.
        "advapi32" => &["ntdll.dll"],
        // P15: DirectSound over door.audio (a software mixer).
        "dsound" => &["ntdll.dll"],
        // P16: COM; the registry through advapi32, the apartments' message-only
        // windows (calls into an STA) through user32.
        "ole32" => &["advapi32.dll", "ntdll.dll", "user32.dll"],
        // P16: the shell: associations and the admin check through
        // advapi32, confirmations and User Account Control through user32.
        "shell32" => &["advapi32.dll", "ntdll.dll", "user32.dll"],
        // P16: Windows Installer; the registry, services and the admin
        // check through advapi32, its dialogs through user32 and gdi32.
        "msi" => &["advapi32.dll", "gdi32.dll", "ntdll.dll", "user32.dll"],
        // P16: OLE Automation (BSTR, VARIANT, SAFEARRAY, IDispatch, type
        // libraries); type-library registration through advapi32, the
        // current directory through kernel32, coclass creation and custom
        // data's task memory through ole32.
        "oleaut32" => &["advapi32.dll", "kernel32.dll", "ntdll.dll", "ole32.dll"],
        // P15 M4: Core Audio over door.audio; endpoint IDs and formats in
        // COM task memory through ole32.
        "mmdevapi" => &["ntdll.dll", "ole32.dll"],
        // P16 M7: the Task Scheduler 2.0 COM API over Init's scheduler; task
        // memory through ole32, BSTRs, VARIANTs and IDispatch through oleaut32.
        "taskschd" => &["ntdll.dll", "ole32.dll", "oleaut32.dll"],
        // P16 M5: WinVerifyTrust and the catalog store; the Trusted Root
        // store through advapi32, CRL retrieval through winhttp.
        "wintrust" => &["advapi32.dll", "ntdll.dll", "winhttp.dll"],
        // P18: the file version API, forwarded to kernel32 (no imports).
        "version" => &[],
        // P18: CNG random numbers (the kernel entropy source) and hashes.
        "bcrypt" => &["ntdll.dll"],
        // P18: the user and computer names through advapi32 and kernel32.
        "secur32" => &["advapi32.dll", "kernel32.dll", "ntdll.dll"],
        // P18: no antimalware provider and no lockdown policy (the process
        // heap is the only thing either could import).
        "amsi" | "wldp" => &["ntdll.dll"],
        // P18: certificate stores, DPAPI keys in the registry (advapi32)
        // and certificate files (kernel32).
        "crypt32" => &["advapi32.dll", "kernel32.dll", "ntdll.dll"],
        // P18: system fonts through gdi32, font files through kernel32.
        "dwrite" => &["gdi32.dll", "kernel32.dll", "ntdll.dll"],
        // P18: ProcessPrng over the kernel entropy source.
        "bcryptprimitives" => &["ntdll.dll"],
        // P18: Personalize settings through advapi32.
        "uxtheme" => &["advapi32.dll", "ntdll.dll"],
        // P18: window rectangles (user32) and the performance counter.
        "dwmapi" => &["kernel32.dll", "ntdll.dll", "user32.dll"],
        // P18: the rest of Chromium's delay-loaded system DLLs. Each answers
        // as Windows does without the hardware, service or GPU behind it
        // (P19 brings the GPU stack); the process heap comes from ntdll.
        "wtsapi32" => &["advapi32.dll", "kernel32.dll", "ntdll.dll", "user32.dll"],
        "shlwapi" => &["advapi32.dll", "kernel32.dll", "ntdll.dll", "ole32.dll"],
        "propsys" => &["ntdll.dll", "ole32.dll", "oleaut32.dll"],
        "userenv" | "rpcrt4" | "dbghelp" | "cryptui" | "setupapi" | "hid" | "winusb" | "mfplat"
        | "wevtapi" | "winspool" | "bthprops" => &["kernel32.dll", "ntdll.dll"],
        "oleacc" | "uiautomationcore" | "pdh" | "tbs" | "ncrypt" | "cfgmgr32" | "mf"
        | "mfreadwrite" | "urlmon" | "dhcpcsvc" | "dxgi" | "d3d11" | "d3d12" | "dcomp" => {
            &["ntdll.dll"]
        }
        _ => return None,
    })
}
/// oleaut32: OleAut32.Lib imports every function by ordinal, so each export
/// the crate defines gets Windows 11's ordinal (`oleaut32-ordinals.txt`,
/// System32\oleaut32.dll's export table). An export is a name the guest
/// sources define with `fn Name(` or list in an export macro
/// (`Name => T`, `Name: type => …`); names the table lacks get free
/// ordinals as before.
fn windows_ordinals(root: &Path, name: &str) -> Result<Vec<(String, u16)>, String> {
    // (table, folder whose .rs files define the exports). ws2_32's classic
    // socket ordinals are `fixed_ordinals`.
    let (table, folder) = match name {
        "oleaut32" => (
            include_str!("oleaut32-ordinals.txt"),
            root.join("compat/dlls/oleaut32/src/guest"),
        ),
        _ => return Ok(Vec::new()),
    };
    let mut files = Vec::new();
    let guest_root = folder.with_extension("rs");
    if guest_root.is_file() {
        files.push(guest_root);
    }
    let mut more: Vec<_> = std::fs::read_dir(&folder)
        .map_err(|e| format!("{}: {e}", folder.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    more.sort();
    files.extend(more);
    let mut defined = std::collections::BTreeSet::new();
    for file in files {
        let text =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        for line in text.lines() {
            let line = line.split("//").next().unwrap_or("");
            exported_names(line, &mut defined);
        }
    }
    let mut out = Vec::new();
    for entry in table
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let (symbol, ordinal) = entry
            .split_once(' ')
            .ok_or_else(|| format!("ordinal table line {entry:?}"))?;
        if defined.contains(symbol) {
            let ordinal = ordinal
                .trim()
                .parse::<u16>()
                .map_err(|e| format!("ordinal of {symbol}: {e}"))?;
            out.push((symbol.to_owned(), ordinal));
        }
    }
    Ok(out)
}
/// The names a source line exports: `fn Name(` and the entries of export
/// macros (`Name => T`, `Name: type => …`).
fn exported_names(line: &str, out: &mut std::collections::BTreeSet<String>) {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut rest = line;
    while let Some(i) = rest.find("fn ") {
        let after = rest[i + 3..].trim_start();
        let n: String = after.chars().take_while(|&c| ident(c)).collect();
        if !n.is_empty() && after[n.len()..].trim_start().starts_with('(') {
            out.insert(n);
        }
        rest = &rest[i + 3..];
    }
    let bytes: Vec<char> = line.chars().collect();
    let mut k = 0;
    while k + 1 < bytes.len() {
        if bytes[k] == '=' && bytes[k + 1] == '>' {
            // Walk back over spaces, an optional `: type`, and the name.
            let mut j = k;
            let back_ident = |j: &mut usize| -> String {
                while *j > 0 && bytes[*j - 1] == ' ' {
                    *j -= 1;
                }
                let end = *j;
                while *j > 0 && ident(bytes[*j - 1]) {
                    *j -= 1;
                }
                bytes[*j..end].iter().collect()
            };
            let mut n = back_ident(&mut j);
            let mut m = j;
            while m > 0 && bytes[m - 1] == ' ' {
                m -= 1;
            }
            if m > 0 && bytes[m - 1] == ':' && (m < 2 || bytes[m - 2] != ':') {
                let mut p = m - 1;
                n = back_ident(&mut p);
            }
            if n.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                out.insert(n);
            }
        }
        k += 1;
    }
}
/// Exports whose ordinals Windows fixes and SDK import libraries import by
/// ordinal (WS2_32.Lib imports the Winsock 1.1 set this way): each DLL
/// exports them at exactly these ordinals; the rest get the next free ones.
/// Exports that are PE forwarders to another DLL, as Windows ships them:
/// x64 kernel32's context, unwinding and function-table calls bind straight
/// to ntdll (RtlCaptureContext must capture its caller, not a wrapper).
pub fn forwarders(name: &str, x86: bool) -> &'static [(&'static str, &'static str)] {
    match (name, x86) {
        ("kernel32", false) => &[
            ("RtlAddFunctionTable", "ntdll.RtlAddFunctionTable"),
            ("RtlCaptureContext", "ntdll.RtlCaptureContext"),
            ("RtlDeleteFunctionTable", "ntdll.RtlDeleteFunctionTable"),
            (
                "RtlInstallFunctionTableCallback",
                "ntdll.RtlInstallFunctionTableCallback",
            ),
            ("RtlPcToFileHeader", "ntdll.RtlPcToFileHeader"),
            ("RtlRestoreContext", "ntdll.RtlRestoreContext"),
            ("RtlUnwind", "ntdll.RtlUnwind"),
            ("RtlUnwindEx", "ntdll.RtlUnwindEx"),
            // Windows' kernel32 forwards the memory helpers too (programs
            // reach them by P/Invoke).
            ("RtlFillMemory", "ntdll.RtlFillMemory"),
            ("RtlMoveMemory", "ntdll.RtlMoveMemory"),
            ("RtlZeroMemory", "ntdll.RtlZeroMemory"),
            // The back trace starts at its caller (no wrapper frame).
            ("RtlCaptureStackBackTrace", "ntdll.RtlCaptureStackBackTrace"),
        ],
        ("kernel32", true) => &[("RtlCaptureStackBackTrace", "ntdll.RtlCaptureStackBackTrace")],
        // msvcrt's C exception handler is ntdll's, as on Windows.
        ("msvcrt", false) => &[("__C_specific_handler", "ntdll.__C_specific_handler")],
        // shell32 hosts the shcore scaling contract (api-ms-win-shcore-scaling,
        // shcore.dll); its DPI answers are user32's.
        ("shell32", _) => &[
            ("GetDpiForMonitor", "user32.GetDpiForMonitor"),
            ("GetProcessDpiAwareness", "user32.GetProcessDpiAwareness"),
            (
                "GetScaleFactorForMonitor",
                "user32.GetScaleFactorForMonitor",
            ),
            ("SetProcessDpiAwareness", "user32.SetProcessDpiAwareness"),
        ],
        // version.dll forwards to the api-ms-win-core-version contract,
        // which kernel32 hosts (Windows: kernelbase).
        ("version", _) => &[
            ("GetFileVersionInfoA", "kernel32.GetFileVersionInfoA"),
            ("GetFileVersionInfoExA", "kernel32.GetFileVersionInfoExA"),
            ("GetFileVersionInfoExW", "kernel32.GetFileVersionInfoExW"),
            (
                "GetFileVersionInfoSizeA",
                "kernel32.GetFileVersionInfoSizeA",
            ),
            (
                "GetFileVersionInfoSizeExA",
                "kernel32.GetFileVersionInfoSizeExA",
            ),
            (
                "GetFileVersionInfoSizeExW",
                "kernel32.GetFileVersionInfoSizeExW",
            ),
            (
                "GetFileVersionInfoSizeW",
                "kernel32.GetFileVersionInfoSizeW",
            ),
            ("GetFileVersionInfoW", "kernel32.GetFileVersionInfoW"),
            ("VerLanguageNameA", "kernel32.VerLanguageNameA"),
            ("VerLanguageNameW", "kernel32.VerLanguageNameW"),
            ("VerQueryValueA", "kernel32.VerQueryValueA"),
            ("VerQueryValueW", "kernel32.VerQueryValueW"),
        ],
        _ => &[],
    }
}
pub fn fixed_ordinals(name: &str) -> &'static [(&'static str, u16)] {
    match name {
        // The dark-mode exports Windows has by ordinal only.
        "uxtheme" => &uxtheme::ORDINALS,
        "ws2_32" => &[
            ("accept", 1),
            ("bind", 2),
            ("closesocket", 3),
            ("connect", 4),
            ("getpeername", 5),
            ("getsockname", 6),
            ("getsockopt", 7),
            ("htonl", 8),
            ("htons", 9),
            ("ioctlsocket", 10),
            ("inet_addr", 11),
            ("inet_ntoa", 12),
            ("listen", 13),
            ("ntohl", 14),
            ("ntohs", 15),
            ("recv", 16),
            ("recvfrom", 17),
            ("select", 18),
            ("send", 19),
            ("sendto", 20),
            ("setsockopt", 21),
            ("shutdown", 22),
            ("socket", 23),
            ("gethostbyaddr", 51),
            ("gethostbyname", 52),
            ("getprotobyname", 53),
            ("getprotobynumber", 54),
            ("getservbyname", 55),
            ("getservbyport", 56),
            ("gethostname", 57),
            ("WSAAsyncSelect", 101),
            ("WSAAsyncGetHostByAddr", 102),
            ("WSAAsyncGetHostByName", 103),
            ("WSAAsyncGetProtoByNumber", 104),
            ("WSAAsyncGetProtoByName", 105),
            ("WSAAsyncGetServByPort", 106),
            ("WSAAsyncGetServByName", 107),
            ("WSACancelAsyncRequest", 108),
            ("WSASetBlockingHook", 109),
            ("WSAUnhookBlockingHook", 110),
            ("WSAGetLastError", 111),
            ("WSASetLastError", 112),
            ("WSACancelBlockingCall", 113),
            ("WSAIsBlocking", 114),
            ("WSAStartup", 115),
            ("WSACleanup", 116),
            ("__WSAFDIsSet", 151),
            ("WEP", 500),
        ],
        // dsound.lib imports every export by ordinal.
        "dsound" => &[
            ("DirectSoundCreate", 1),
            ("DirectSoundEnumerateA", 2),
            ("DirectSoundEnumerateW", 3),
            ("DllCanUnloadNow", 4),
            ("DllGetClassObject", 5),
            ("DirectSoundCaptureCreate", 6),
            ("DirectSoundCaptureEnumerateA", 7),
            ("DirectSoundCaptureEnumerateW", 8),
            ("GetDeviceID", 9),
            ("DirectSoundFullDuplexCreate", 10),
            ("DirectSoundCreate8", 11),
            ("DirectSoundCaptureCreate8", 12),
        ],
        // shlwapi's ordinal-only imports, d3d12's and winspool's numbered exports.
        "shlwapi" => &[("SHCreateMemStream", 12), ("QISearch", 219)],
        "d3d12" => &[
            ("D3D12CreateDevice", 101),
            ("D3D12GetDebugInterface", 102),
            ("D3D12EnableExperimentalFeatures", 109),
            ("D3D12GetInterface", 110),
            ("D3D12SerializeRootSignature", 115),
            ("D3D12SerializeVersionedRootSignature", 116),
        ],
        "winspool" => &[("GetDefaultPrinterW", 203)],
        // Chromium imports ActivateAudioInterfaceAsync by ordinal.
        "mmdevapi" => &[("ActivateAudioInterfaceAsync", 17)],
        // comctl32 v6's task dialogs, which Chromium imports by ordinal.
        "comctl32" => &[("TaskDialog", 344), ("TaskDialogIndirect", 345)],
        // shell32.lib imports these by ordinal.
        "shell32" => &[
            ("SHChangeNotifyRegister", 2),
            ("SHChangeNotifyDeregister", 4),
            ("ILFindLastID", 16),
            ("ILRemoveLastID", 17),
            ("ILClone", 18),
            ("ILCombine", 25),
            ("ILGetSize", 152),
            ("ILFree", 155),
            ("SHCreateDirectory", 165),
            ("ILCreateFromPathW", 190),
            ("SHFree", 195),
            ("SHAlloc", 196),
            ("IsUserAnAdmin", 680),
        ],
        // Msi.Lib imports every export by ordinal; Doors' own entry point
        // sits past Windows' range.
        "msi" => &[
            ("MsiCloseAllHandles", 7),
            ("MsiCloseHandle", 8),
            ("MsiConfigureProductW", 16),
            ("MsiCreateRecord", 17),
            ("MsiDatabaseGetPrimaryKeysW", 26),
            ("MsiDatabaseOpenViewA", 31),
            ("MsiDatabaseOpenViewW", 32),
            ("MsiDoActionW", 34),
            ("MsiEnumProductsW", 45),
            ("MsiEvaluateConditionW", 47),
            ("MsiGetActiveDatabase", 49),
            ("MsiGetLanguage", 63),
            ("MsiGetMode", 64),
            ("MsiGetProductCodeW", 66),
            ("MsiGetProductInfoA", 67),
            ("MsiGetProductInfoW", 70),
            ("MsiGetPropertyA", 73),
            ("MsiGetPropertyW", 74),
            ("MsiGetSourcePathW", 76),
            ("MsiGetSummaryInformationW", 78),
            ("MsiGetTargetPathW", 80),
            ("MsiInstallProductA", 87),
            ("MsiInstallProductW", 88),
            ("MsiLocateComponentW", 90),
            ("MsiOpenDatabaseA", 91),
            ("MsiOpenDatabaseW", 92),
            ("MsiOpenPackageW", 94),
            ("MsiProcessMessage", 103),
            ("MsiQueryProductStateA", 112),
            ("MsiQueryProductStateW", 113),
            ("MsiRecordDataSize", 114),
            ("MsiRecordGetFieldCount", 115),
            ("MsiRecordGetInteger", 116),
            ("MsiRecordGetStringA", 117),
            ("MsiRecordGetStringW", 118),
            ("MsiRecordIsNull", 119),
            ("MsiRecordReadStream", 120),
            ("MsiRecordSetInteger", 121),
            ("MsiRecordSetStringA", 124),
            ("MsiRecordSetStringW", 125),
            ("MsiReinstallProductW", 131),
            ("MsiSetExternalUIW", 137),
            ("MsiSetInternalUI", 141),
            ("MsiSetPropertyA", 144),
            ("MsiSetPropertyW", 145),
            ("MsiSetTargetPathW", 147),
            ("MsiSummaryInfoGetPropertyCount", 149),
            ("MsiSummaryInfoGetPropertyW", 150),
            ("MsiViewClose", 158),
            ("MsiViewExecute", 159),
            ("MsiViewFetch", 160),
            ("MsiViewGetColumnInfo", 166),
            ("MsiRecordClearData", 167),
            ("MsiEnableLogW", 169),
            ("MsiFormatRecordW", 171),
            ("MsiGetComponentPathW", 173),
            ("MsiConfigureProductExW", 190),
            ("MsiGetFileVersionW", 195),
            ("MsiEnumRelatedProductsW", 205),
            ("MsiOpenPackageExW", 232),
            ("DoorMsiexecMain", 1000),
        ],
        _ => &[],
    }
}
/// Lower-case, sorted module names a PE inspection reports.
pub fn imported_modules(pe: &Value) -> Result<Vec<String>, String> {
    let mut modules: Vec<String> = pe["imports"]
        .as_array()
        .ok_or("DLL imports")?
        .iter()
        .map(|m| m["module"].as_str().unwrap_or("").to_ascii_lowercase())
        .collect();
    modules.sort();
    Ok(modules)
}
/// DLLs with an entry point (DllMain for DLL_THREAD_DETACH: user32 frees a
/// dying thread's windows and queue, ole32 its apartment).
pub fn has_entry(name: &str) -> bool {
    matches!(name, "user32" | "ole32")
}
/// The DLL imports exactly its declared graph and has an entry point only
/// when `has_entry` says so.
pub fn check_graph(name: &str, pe: &Value) -> Result<(), String> {
    let allowed = allowed_imports(name).ok_or("unknown original DLL")?;
    let actual = imported_modules(pe)?;
    if actual != allowed || (pe["entry_rva"] != 0) != has_entry(name) {
        return Err(format!(
            "{name}.dll imports {actual:?} (declared {allowed:?}) or has an entry point"
        ));
    }
    Ok(())
}
/// The RUSTFLAGS every original x64 DLL builds with (`CARGO_ENCODED_RUSTFLAGS`).
pub fn flags(root: &Path, target: &Path, name: &str) -> Result<String, String> {
    Ok(flags_for(root, target, name, false)?.0)
}
/// `flags` for the x64 build, or the 32-bit one (`x86`), and the DLL's own
/// link arguments (entry point, exports, base, import library and map),
/// which go to the final `rustc` only (`cargo rustc -- ...`): the shared
/// dependencies then build with the same flags for every DLL, so they
/// compile once instead of once per DLL.
fn flags_for(
    root: &Path,
    target: &Path,
    name: &str,
    x86: bool,
) -> Result<(String, Vec<String>), String> {
    let profile = crate::image::read_json(&root.join(super::build::PROFILE))?;
    let imports = if x86 {
        "windows-imports32"
    } else {
        "windows-imports"
    };
    let base = format!("{:#x}", if x86 { base32(name)? } else { base64(name)? });
    let mut flags = vec![
        format!("--remap-path-prefix={}=/door-os", root.display()),
        "-C".into(),
        format!(
            "linker={}",
            profile["dll_linker"].as_str().ok_or("pinned DLL linker")?
        ),
        "-C".into(),
        "panic=abort".into(),
        "-C".into(),
        "lto=off".into(),
        "-C".into(),
        "relocation-model=pic".into(),
        "-L".into(),
        format!("native={}", super::native_path(&target.join(imports))?),
    ];
    for flag in [
        "/NODEFAULTLIB",
        "/DEBUG:NONE",
        "/Brepro",
        "/DYNAMICBASE",
        "/FIXED:NO",
        "/NXCOMPAT",
        "/SUBSYSTEM:CONSOLE",
        "/INCREMENTAL:NO",
    ] {
        flags.extend(["-C".into(), format!("link-arg={flag}")]);
    }
    // The DLL's own link arguments. user32 and ole32 need DllMain for
    // DLL_THREAD_DETACH (a dying thread's windows and queue; its
    // apartment); the others have no entry.
    let mut own: Vec<String> = Vec::new();
    let entry = if has_entry(name) {
        "/ENTRY:DllMain"
    } else {
        "/NOENTRY"
    };
    own.extend(["-C".into(), format!("link-arg={entry}")]);
    for (symbol, ordinal) in fixed_ordinals(name) {
        own.extend(["-C".into(), format!("link-arg=/EXPORT:{symbol},@{ordinal}")]);
    }
    for (symbol, ordinal) in windows_ordinals(root, name)? {
        own.extend(["-C".into(), format!("link-arg=/EXPORT:{symbol},@{ordinal}")]);
    }
    for (symbol, target) in forwarders(name, x86) {
        own.extend(["-C".into(), format!("link-arg=/EXPORT:{symbol}={target}")]);
    }
    for flag in [
        format!("/BASE:{base}"),
        format!(
            "/IMPLIB:{}",
            super::native_path(&target.join(format!("{imports}/{name}.lib")))?
        ),
        format!(
            "/MAP:{}",
            super::native_path(&target.join(if x86 {
                format!("{name}-x86.map")
            } else {
                format!("{name}.map")
            }))?
        ),
    ] {
        own.extend(["-C".into(), format!("link-arg={flag}")]);
    }
    Ok((flags.join("\x1f"), own))
}
/// The cargo arguments of an original x64 DLL's build, its own link
/// arguments after `--`.
pub fn argv(root: &Path, target: &Path, name: &str) -> Result<Vec<String>, String> {
    let mut args = argv_for(target, name, "x86_64-pc-windows-msvc")?;
    args.push("--".into());
    args.extend(flags_for(root, target, name, false)?.1);
    Ok(args)
}
fn argv_for(target: &Path, name: &str, triple: &str) -> Result<Vec<String>, String> {
    if !NAMES.contains(&name) && name != "msvcrt" {
        return Err("unknown original DLL package".into());
    }
    // door-msi is the database crate; the DLL is door-msi-dll.
    let package = if name == "msi" {
        String::from("door-msi-dll")
    } else {
        format!("door-{name}")
    };
    Ok([
        "rustc",
        "--locked",
        "--release",
        "--package",
        &package,
        "--lib",
        "--crate-type",
        "cdylib",
        "--features",
        "guest-pe",
        "--target",
        triple,
        "--target-dir",
        target.to_str().ok_or("DLL build target")?,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect())
}
pub fn build(run: &mut Run, target: &Path) -> Result<Vec<Value>, String> {
    let mut artifacts = build_names(run, target, &NAMES)?;
    artifacts.extend(build_programs(run, target)?);
    artifacts.extend(build_names32(run, target, &NAMES32)?);
    artifacts.extend(build_programs_for(run, target, true)?);
    Ok(artifacts)
}
/// The 32-bit builds (P17): artifacts `<name>-x86`, staged at
/// `windows32/<name>.dll`, with the import graph of the x64 builds.
fn build_names32(run: &mut Run, target: &Path, names: &[&str]) -> Result<Vec<Value>, String> {
    super::build::toolchain(&run.root)?;
    fs::create_dir_all(target.join("windows-imports32")).map_err(|e| e.to_string())?;
    let mut artifacts = Vec::new();
    for &name in names {
        let (flags, own) = flags_for(&run.root, target, name, true)?;
        let mut args = argv_for(target, name, "i686-pc-windows-msvc")?;
        args.push("--".into());
        args.extend(own);
        let observed = run.bounded(
            "cargo",
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            BUILD_BOUND,
            &[
                ("CARGO_ENCODED_RUSTFLAGS", &flags),
                ("CARGO_PROFILE_RELEASE_LTO", "false"),
            ],
            true,
        )?;
        if !observed.success {
            return Err(format!("original 32-bit {name}.dll build failed"));
        }
        let path = target.join(format!("i686-pc-windows-msvc/release/{name}.dll"));
        let map = target.join(format!("{name}-x86.map"));
        let import = target.join(format!("windows-imports32/{name}.lib"));
        let inspected = artifact::inspect(&path, "windows-dll")?;
        let metadata = super::pe::inspect(&path)?;
        check_graph(name, &metadata)?;
        let extent = metadata["sections"]
            .as_array()
            .ok_or("x86 DLL sections")?
            .iter()
            .map(|s| s["rva"].as_u64().unwrap_or(0) + s["virtual_bytes"].as_u64().unwrap_or(0))
            .max()
            .unwrap_or(0);
        if extent > PROVIDER32_BYTES {
            return Err(format!(
                "32-bit {name}.dll maps {extent:#x} bytes, more than its slot"
            ));
        }
        artifacts.push(json!({"name":format!("{name}-x86"),"path":relative(&run.root,&path)?,"artifact":inspected,"pe":metadata,"build_inputs":run.build_inputs(),"initfs_destination":format!("windows32/{}", file_name(name)),"link_map":{"path":relative(&run.root,&map)?,"sha256":file_hash(&map)?},"import_library":{"path":relative(&run.root,&import)?,"sha256":file_hash(&import)?},"runtime":"original Windows ABI adapter (32-bit, WOW64); native Door services","build_command":run.commands().last().ok_or("DLL build command")?["id"]}));
    }
    Ok(artifacts)
}
/// msiexec.exe and the other original programs: Rust bins for the Windows
/// target linked against the original DLLs' import libraries.
pub fn build_programs(run: &mut Run, target: &Path) -> Result<Vec<Value>, String> {
    build_programs_for(run, target, false)
}
/// `build_programs` for x64, or the 32-bit builds (`x86`, P17): artifacts
/// `<name>-x86` staged at `seed/<name>32.exe` and installed into
/// C:\Windows\SysWOW64 (Windows Installer runs a 32-bit package's
/// installation, and its DLL custom actions, in SysWOW64\msiexec.exe).
fn build_programs_for(run: &mut Run, target: &Path, x86: bool) -> Result<Vec<Value>, String> {
    let profile = crate::image::read_json(&run.root.join(super::build::PROFILE))?;
    let (triple, imports_dir, base) = if x86 {
        (
            "i686-pc-windows-msvc",
            "windows-imports32",
            "/BASE:0x400000",
        )
    } else {
        (
            "x86_64-pc-windows-msvc",
            "windows-imports",
            "/BASE:0x142000000",
        )
    };
    let mut artifacts = Vec::new();
    for (name, imports) in PROGRAMS {
        let mut flags = vec![
            format!("--remap-path-prefix={}=/door-os", run.root.display()),
            "-C".into(),
            format!(
                "linker={}",
                profile["dll_linker"].as_str().ok_or("pinned DLL linker")?
            ),
            "-C".into(),
            "panic=abort".into(),
            "-L".into(),
            format!("native={}", super::native_path(&target.join(imports_dir))?),
        ];
        // A program's version resource (`compat/programs/<name>/<name>.rc`),
        // compiled with the pinned resource compiler and linked in.
        let script = run
            .root
            .join("compat/programs")
            .join(name)
            .join(format!("{name}.rc"));
        let mut resource = None;
        if script.exists() {
            let msvc = profile["msvc_root"].as_str().ok_or("MSVC root")?;
            let sdk = profile["sdk_root"].as_str().ok_or("SDK root")?;
            let version = profile["sdk_version"].as_str().ok_or("SDK version")?;
            fs::create_dir_all(target).map_err(|e| e.to_string())?;
            let res = target.join(if x86 {
                format!("{name}-x86.res")
            } else {
                format!("{name}.res")
            });
            let args = [
                "/nologo".to_string(),
                "/x".into(),
                format!("/I{sdk}/Include/{version}/um"),
                format!("/I{sdk}/Include/{version}/shared"),
                format!("/I{msvc}/include"),
                format!("/fo{}", super::native_path(&res)?),
                super::native_path(&script)?,
            ];
            run.required(
                profile["resource_compiler"]
                    .as_str()
                    .ok_or("pinned resource compiler")?,
                &args.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
            flags.extend([
                "-C".into(),
                format!("link-arg={}", super::native_path(&res)?),
            ]);
            resource = Some(
                json!({"script":relative(&run.root,&script)?,"script_sha256":file_hash(&script)?,
                "res":relative(&run.root,&res)?,"res_sha256":file_hash(&res)?}),
            );
        }
        for flag in [
            format!("/ENTRY:{name}_main"),
            "/NODEFAULTLIB".into(),
            "/DEBUG:NONE".into(),
            "/Brepro".into(),
            "/DYNAMICBASE".into(),
            "/NXCOMPAT".into(),
            "/SUBSYSTEM:WINDOWS".into(),
            "/INCREMENTAL:NO".into(),
            base.into(),
            format!(
                "/MAP:{}",
                super::native_path(&target.join(if x86 {
                    format!("{name}-x86.map")
                } else {
                    format!("{name}.map")
                }))?
            ),
        ] {
            flags.extend(["-C".into(), format!("link-arg={flag}")]);
        }
        let package = format!("door-{name}");
        let args = [
            "rustc",
            "--locked",
            "--release",
            "--package",
            package.as_str(),
            "--bin",
            name,
            "--features",
            "guest-pe",
            "--target",
            triple,
            "--target-dir",
            target.to_str().ok_or("program build target")?,
        ];
        let observed = run.bounded(
            "cargo",
            &args,
            BUILD_BOUND,
            &[
                ("CARGO_ENCODED_RUSTFLAGS", &flags.join("\x1f")),
                ("CARGO_PROFILE_RELEASE_LTO", "false"),
            ],
            true,
        )?;
        if !observed.success {
            return Err(format!(
                "original {}{name}.exe build failed",
                if x86 { "32-bit " } else { "" }
            ));
        }
        let path = target.join(format!("{triple}/release/{name}.exe"));
        let inspected = artifact::inspect(&path, "windows-fixture")?;
        let metadata = super::pe::inspect(&path)?;
        let actual = imported_modules(&metadata)?;
        if actual != *imports {
            return Err(format!(
                "{name}.exe imports {actual:?} (declared {imports:?})"
            ));
        }
        let (artifact_name, destination, runtime) = if x86 {
            (
                format!("{name}-x86"),
                format!("seed/{name}32.exe"),
                "original Windows program (32-bit, WOW64); installed into C:\\Windows\\SysWOW64",
            )
        } else {
            (
                String::from(name),
                format!("seed/{name}.exe"),
                "original Windows program; installed into C:\\Windows\\System32",
            )
        };
        artifacts.push(json!({"name":artifact_name,"path":relative(&run.root,&path)?,"artifact":inspected,"pe":metadata,"build_inputs":run.build_inputs(),"initfs_destination":destination,"runtime":runtime,"resource":resource,"build_command":run.commands().last().ok_or("program build command")?["id"]}));
    }
    Ok(artifacts)
}
pub fn build_crt(run: &mut Run, target: &Path) -> Result<Vec<Value>, String> {
    build_names(run, target, &["msvcrt"])
}
/// The Windows GUI suites rebuild every original DLL (the GUI DLLs link
/// against the base import libraries of the same target).
/// The bound on one DLL's or program's cargo build. Each run builds into a
/// fresh target directory, and gdi32 with its font crates took over 200 s
/// (win32-gui-f1, 11 November 2026): a guard against a hung compiler, not
/// a performance check.
pub(crate) const BUILD_BOUND: Duration = Duration::from_secs(600);
pub fn build_gui(run: &mut Run, target: &Path) -> Result<Vec<Value>, String> {
    build_names(run, target, &NAMES)
}
fn build_names(run: &mut Run, target: &Path, names: &[&str]) -> Result<Vec<Value>, String> {
    super::build::toolchain(&run.root)?;
    fs::create_dir_all(target.join("windows-imports")).map_err(|e| e.to_string())?;
    let mut artifacts = Vec::new();
    for &name in names {
        let flags = flags(&run.root, target, name)?;
        let mut args = argv(&run.root, target, name)?;
        // A DLL's resources (`compat/dlls/<name>/<name>.rc`: taskschd's type
        // library), compiled with the pinned resource compiler and linked in.
        let script = run
            .root
            .join("compat/dlls")
            .join(name)
            .join(format!("{name}.rc"));
        let mut resource = None;
        if script.exists() {
            let profile = crate::image::read_json(&run.root.join(super::build::PROFILE))?;
            let msvc = profile["msvc_root"].as_str().ok_or("MSVC root")?;
            let sdk = profile["sdk_root"].as_str().ok_or("SDK root")?;
            let version = profile["sdk_version"].as_str().ok_or("SDK version")?;
            let res = target.join(format!("{name}.res"));
            let rc = [
                "/nologo".to_string(),
                "/x".into(),
                format!("/I{sdk}/Include/{version}/um"),
                format!("/I{sdk}/Include/{version}/shared"),
                format!("/I{msvc}/include"),
                format!("/fo{}", super::native_path(&res)?),
                super::native_path(&script)?,
            ];
            run.required(
                profile["resource_compiler"]
                    .as_str()
                    .ok_or("pinned resource compiler")?,
                &rc.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
            args.extend([
                "-C".into(),
                format!("link-arg={}", super::native_path(&res)?),
            ]);
            resource = Some(
                json!({"script":relative(&run.root,&script)?,"script_sha256":file_hash(&script)?,"res":relative(&run.root,&res)?,"res_sha256":file_hash(&res)?}),
            );
        }
        let observed = run.bounded(
            "cargo",
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            BUILD_BOUND,
            &[
                ("CARGO_ENCODED_RUSTFLAGS", &flags),
                ("CARGO_PROFILE_RELEASE_LTO", "false"),
            ],
            true,
        )?;
        if !observed.success {
            return Err(format!("original {name}.dll build failed"));
        }
        let path = target.join(format!("x86_64-pc-windows-msvc/release/{name}.dll"));
        let map = target.join(format!("{name}.map"));
        let import = target.join(format!("windows-imports/{name}.lib"));
        let inspected = artifact::inspect(&path, "windows-dll")?;
        let metadata = super::pe::inspect(&path)?;
        check_graph(name, &metadata)?;
        if (inspected["inspection"]["entry_rva"] != 0) != has_entry(name) {
            return Err(format!("{name}.dll has an unsupported entry point"));
        }
        artifacts.push(json!({"name":name,"path":relative(&run.root,&path)?,"artifact":inspected,"pe":metadata,"build_inputs":run.build_inputs(),"initfs_destination":format!("windows/{}", file_name(name)),"link_map":{"path":relative(&run.root,&map)?,"sha256":file_hash(&map)?},"import_library":{"path":relative(&run.root,&import)?,"sha256":file_hash(&import)?},"runtime":"original Windows ABI adapter; native Door services","resource":resource,"build_command":run.commands().last().ok_or("DLL build command")?["id"]}));
    }
    Ok(artifacts)
}
