//! System COM class registrations: HKLM\Software\Classes\CLSID entries for
//! the classes Doors' own DLLs serve, as Windows setup ships them in the
//! registry. Init writes them (machine-wide, system access) when it
//! installs the userland, so ole32's CoCreateInstance finds each server
//! through InprocServer32 like any registered class.
use door_service_client::{RegistryRoot, RegistryValue, RegistryView, Storage};

/// (CLSID, the class's name, its server). Windows registers these servers
/// as REG_EXPAND_SZ "%SystemRoot%\System32\...", as here.
const CLASSES: [(&str, &str, &str); 9] = [
    (
        "{47D4D946-62E8-11CF-93BC-444553540000}",
        "DirectSound Object",
        DSOUND,
    ),
    (
        "{3901CC3F-84B5-4FA4-BA35-AA8172B8A09B}",
        "DirectSound 8.0 Object",
        DSOUND,
    ),
    (
        "{B0210780-89CD-11D0-AF08-00A0C925CD16}",
        "DirectSoundCapture Object",
        DSOUND,
    ),
    (
        "{E4BCAC13-7F99-4908-9A8E-74E3BF24B6E1}",
        "DirectSoundCapture 8.0 Object",
        DSOUND,
    ),
    (
        "{FEA4300C-7959-4147-B26A-2377B9E7A91D}",
        "DirectSoundFullDuplex Object",
        DSOUND,
    ),
    // The automation marshalers ole32 asks for an interface's proxy and
    // stub (Interface\{iid}\ProxyStubClsid32).
    (
        "{00020424-0000-0000-C000-000000000046}",
        "PSOAInterface",
        OLEAUT32,
    ),
    (
        "{00020420-0000-0000-C000-000000000046}",
        "PSDispatch",
        OLEAUT32,
    ),
    // Core Audio's endpoint enumerator (P15 M4).
    (
        "{BCDE0395-E52F-467C-8E3D-C4579291692E}",
        "MMDeviceEnumerator class",
        MMDEVAPI,
    ),
    // The Task Scheduler 2.0 API (P16 M7).
    (
        "{0F87369F-A4E5-4CFC-BD3E-73E6154572DD}",
        "Task Scheduler 2.0",
        TASKSCHD,
    ),
];
const DSOUND: &str = r"%SystemRoot%\System32\dsound.dll";
const OLEAUT32: &str = r"%SystemRoot%\System32\oleaut32.dll";
const MMDEVAPI: &str = r"%SystemRoot%\System32\MMDevApi.dll";
const TASKSCHD: &str = r"%SystemRoot%\System32\taskschd.dll";
/// IDispatch's marshaling, stdole2's and taskschd.dll's type libraries'
/// registration, as Windows setup ships them.
const AUTOMATION: [(&str, &str, u8, &str); 11] = [
    (
        r"Software\Classes\Interface\{00020400-0000-0000-C000-000000000046}",
        "",
        REG_SZ,
        "IDispatch",
    ),
    (
        r"Software\Classes\Interface\{00020400-0000-0000-C000-000000000046}\ProxyStubClsid32",
        "",
        REG_SZ,
        "{00020420-0000-0000-C000-000000000046}",
    ),
    (
        r"Software\Classes\Interface\{00020400-0000-0000-C000-000000000046}\ProxyStubClsid",
        "",
        REG_SZ,
        "{00020420-0000-0000-C000-000000000046}",
    ),
    (
        r"Software\Classes\TypeLib\{00020430-0000-0000-C000-000000000046}\2.0",
        "",
        REG_SZ,
        "OLE Automation",
    ),
    (
        r"Software\Classes\TypeLib\{00020430-0000-0000-C000-000000000046}\2.0\FLAGS",
        "",
        REG_SZ,
        "0",
    ),
    (
        r"Software\Classes\TypeLib\{00020430-0000-0000-C000-000000000046}\2.0\HELPDIR",
        "",
        REG_SZ,
        r"C:\Windows\system32",
    ),
    (
        r"Software\Classes\TypeLib\{00020430-0000-0000-C000-000000000046}\2.0\0\win64",
        "",
        REG_SZ,
        r"C:\Windows\System32\stdole2.tlb",
    ),
    // taskschd.dll's TaskScheduler 1.0 library (its TYPELIB resource).
    (
        r"Software\Classes\TypeLib\{E34CB9F1-C7F7-424C-BE29-027DCC09363A}\1.0",
        "",
        REG_SZ,
        "TaskScheduler",
    ),
    (
        r"Software\Classes\TypeLib\{E34CB9F1-C7F7-424C-BE29-027DCC09363A}\1.0\FLAGS",
        "",
        REG_SZ,
        "0",
    ),
    (
        r"Software\Classes\TypeLib\{E34CB9F1-C7F7-424C-BE29-027DCC09363A}\1.0\HELPDIR",
        "",
        REG_SZ,
        r"C:\Windows\system32",
    ),
    (
        r"Software\Classes\TypeLib\{E34CB9F1-C7F7-424C-BE29-027DCC09363A}\1.0\0\win64",
        "",
        REG_SZ,
        r"C:\Windows\System32\taskschd.dll",
    ),
];
const CLSID_KEY: &str = r"Software\Classes\CLSID\";
const INPROC: &str = r"\InprocServer32";
const KEY_ALL: u16 = 3;
const REG_SZ: u8 = 1;
const REG_EXPAND_SZ: u8 = 2;

/// `parts` joined, as UTF-16 (registry paths hold 96 units).
fn units(parts: &[&str]) -> ([u16; 96], usize) {
    let mut out = [0u16; 96];
    let mut len = 0;
    for (slot, unit) in out
        .iter_mut()
        .zip(parts.iter().flat_map(|p| p.encode_utf16()))
    {
        *slot = unit;
        len += 1;
    }
    (out, len)
}
/// String data: UTF-16LE with its terminating NUL.
fn string(text: &str, out: &mut [u8; 256]) -> usize {
    let mut len = 0;
    for unit in text.encode_utf16().chain([0]) {
        out[len..len + 2].copy_from_slice(&unit.to_le_bytes());
        len += 2;
    }
    len
}
/// Sets `values` ((name, type, data); "" is the default value) on the
/// HKLM key `path` joins to, creating it.
fn set(
    storage: &mut Storage,
    path: &[&str],
    values: &[(&str, u8, &str)],
) -> door_service_client::Result<()> {
    let (path, path_len) = units(path);
    let key = storage.registry_open(
        RegistryRoot::LocalMachine,
        RegistryView::Bits64,
        &path[..path_len],
        KEY_ALL,
        true,
    )?;
    // Registered at every sign-in: only a missing or different value is
    // written, so an unchanged session leaves the hive (and the volume)
    // byte-identical, as the program install above leaves unchanged files.
    let written = values.iter().try_for_each(|(name, kind, data)| {
        let (name, name_len) = units(&[name]);
        let mut bytes = [0u8; 256];
        let len = string(data, &mut bytes);
        let value = RegistryValue::new(*kind, &bytes[..len])?;
        match storage.registry_query(&key, &name[..name_len]) {
            Ok(current) if current.kind == value.kind && current.data() == value.data() => Ok(()),
            _ => storage.registry_set(&key, &name[..name_len], value),
        }
    });
    let closed = storage.registry_close(key);
    written.and(closed)
}
/// Windows Installer's file types (.msi packages, .msp patches): the keys
/// and values Windows setup ships, so ShellExecute and File Explorer run
/// msiexec.exe with the verb's switch.
const INSTALLER: [(&str, &str, u8, &str); 12] = [
    (r"Software\Classes\.msi", "", REG_SZ, "Msi.Package"),
    (
        r"Software\Classes\.msi",
        "Content Type",
        REG_SZ,
        "application/x-ole-storage",
    ),
    (
        r"Software\Classes\Msi.Package",
        "",
        REG_SZ,
        "Windows Installer Package",
    ),
    (
        r"Software\Classes\Msi.Package\DefaultIcon",
        "",
        REG_EXPAND_SZ,
        r"%SystemRoot%\System32\msiexec.exe,0",
    ),
    (
        r"Software\Classes\Msi.Package\shell\Open",
        "",
        REG_SZ,
        "&Install",
    ),
    (
        r"Software\Classes\Msi.Package\shell\Open\command",
        "",
        REG_EXPAND_SZ,
        r#""%SystemRoot%\System32\msiexec.exe" /i "%1" %*"#,
    ),
    (
        r"Software\Classes\Msi.Package\shell\repair",
        "",
        REG_SZ,
        "Re&pair",
    ),
    (
        r"Software\Classes\Msi.Package\shell\repair\command",
        "",
        REG_EXPAND_SZ,
        r#""%SystemRoot%\System32\msiexec.exe" /f "%1" %*"#,
    ),
    (
        r"Software\Classes\Msi.Package\shell\Uninstall",
        "",
        REG_SZ,
        "&Uninstall",
    ),
    (
        r"Software\Classes\Msi.Package\shell\Uninstall\command",
        "",
        REG_EXPAND_SZ,
        r#""%SystemRoot%\System32\msiexec.exe" /x "%1" %*"#,
    ),
    (r"Software\Classes\.msp", "", REG_SZ, "Msi.Patch"),
    (
        r"Software\Classes\Msi.Patch",
        "",
        REG_SZ,
        "Windows Installer Patch",
    ),
];
const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion";
const CURRENT_VERSION_32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion";
/// The folder values of HKLM\...\Windows\CurrentVersion in the 64-bit view
/// and, below WOW6432Node, the 32-bit view (P17), as Windows 11 26H2 holds
/// them (measured with `reg query ... /reg:64` and `/reg:32`): programs and
/// installers read Program Files and Common Files from here, a 32-bit one
/// getting the (x86) folders.
const FOLDERS: [(&str, &str, u8, &str); 18] = [
    (
        CURRENT_VERSION,
        "ProgramFilesDir",
        REG_SZ,
        r"C:\Program Files",
    ),
    (
        CURRENT_VERSION,
        "CommonFilesDir",
        REG_SZ,
        r"C:\Program Files\Common Files",
    ),
    (
        CURRENT_VERSION,
        "ProgramFilesDir (x86)",
        REG_SZ,
        r"C:\Program Files (x86)",
    ),
    (
        CURRENT_VERSION,
        "CommonFilesDir (x86)",
        REG_SZ,
        r"C:\Program Files (x86)\Common Files",
    ),
    (
        CURRENT_VERSION,
        "CommonW6432Dir",
        REG_SZ,
        r"C:\Program Files\Common Files",
    ),
    (
        CURRENT_VERSION,
        "ProgramW6432Dir",
        REG_SZ,
        r"C:\Program Files",
    ),
    (
        CURRENT_VERSION,
        "DevicePath",
        REG_EXPAND_SZ,
        r"%SystemRoot%\inf",
    ),
    (
        CURRENT_VERSION,
        "MediaPathUnexpanded",
        REG_EXPAND_SZ,
        r"%SystemRoot%\Media",
    ),
    (
        CURRENT_VERSION,
        "ProgramFilesPath",
        REG_EXPAND_SZ,
        "%ProgramFiles%",
    ),
    (
        CURRENT_VERSION_32,
        "ProgramFilesDir",
        REG_SZ,
        r"C:\Program Files (x86)",
    ),
    (
        CURRENT_VERSION_32,
        "CommonFilesDir",
        REG_SZ,
        r"C:\Program Files (x86)\Common Files",
    ),
    (
        CURRENT_VERSION_32,
        "ProgramFilesDir (x86)",
        REG_SZ,
        r"C:\Program Files (x86)",
    ),
    (
        CURRENT_VERSION_32,
        "CommonFilesDir (x86)",
        REG_SZ,
        r"C:\Program Files (x86)\Common Files",
    ),
    (
        CURRENT_VERSION_32,
        "CommonW6432Dir",
        REG_SZ,
        r"C:\Program Files\Common Files",
    ),
    (
        CURRENT_VERSION_32,
        "ProgramW6432Dir",
        REG_SZ,
        r"C:\Program Files",
    ),
    (
        CURRENT_VERSION_32,
        "DevicePath",
        REG_EXPAND_SZ,
        r"%SystemRoot%\inf",
    ),
    (
        CURRENT_VERSION_32,
        "MediaPathUnexpanded",
        REG_EXPAND_SZ,
        r"%SystemRoot%\Media",
    ),
    (
        CURRENT_VERSION_32,
        "ProgramFilesPath",
        REG_EXPAND_SZ,
        "%ProgramFiles%",
    ),
];
/// HKLM\...\CurrentVersion\App Paths, a key WOW64 shares between its views
/// (P17): Windows ships it, with per-application entries that Run and
/// ShellExecute consult and installers add to. Doors' own applications
/// register none, so it starts empty.
const APP_PATHS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
pub(super) fn register(storage: &mut Storage) -> door_service_client::Result<()> {
    for &(key, name, kind, value) in INSTALLER.iter().chain(&AUTOMATION).chain(&FOLDERS) {
        set(storage, &[key], &[(name, kind, value)])?;
    }
    set(storage, &[APP_PATHS], &[])?;
    for (clsid, name, server) in CLASSES {
        set(storage, &[CLSID_KEY, clsid], &[("", REG_SZ, name)])?;
        set(
            storage,
            &[CLSID_KEY, clsid, INPROC],
            &[
                ("", REG_EXPAND_SZ, server),
                ("ThreadingModel", REG_SZ, "Both"),
            ],
        )?;
    }
    storage.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registrations_fit_the_registry_limits() {
        for (clsid, name, server) in CLASSES {
            assert_eq!(clsid.len(), 38);
            let path: usize = [CLSID_KEY, clsid, INPROC]
                .iter()
                .map(|p| p.encode_utf16().count())
                .sum();
            assert!(path <= 96);
            for text in [name, server] {
                assert!((text.encode_utf16().count() + 1) * 2 <= 256, "{text}");
            }
        }
        for &(key, name, _, value) in INSTALLER.iter().chain(&AUTOMATION).chain(&FOLDERS) {
            assert!(key.encode_utf16().count() <= 96, "{key}");
            assert!(name.encode_utf16().count() <= 96);
            assert!((value.encode_utf16().count() + 1) * 2 <= 256, "{value}");
        }
    }
}
