//! What the engine (`engine`) works on: the file system, registry, services
//! and processes of the machine (`Target`), Windows Installer's exit codes,
//! and the Installer's own records of installed products (`records`).
use crate::install::{RegValue, Root, ServicePlan};
use alloc::{format, string::String, vec::Vec};

/// msiexec and MsiInstallProduct results.
pub mod code {
    pub const SUCCESS: u32 = 0;
    pub const NO_MORE_ITEMS: u32 = 259;
    pub const ELEVATION_REQUIRED: u32 = 740;
    pub const INSTALL_USEREXIT: u32 = 1602;
    pub const INSTALL_FAILURE: u32 = 1603;
    pub const INSTALL_SUSPEND: u32 = 1604;
    pub const UNKNOWN_PRODUCT: u32 = 1605;
    pub const UNKNOWN_FEATURE: u32 = 1606;
    pub const BAD_CONFIGURATION: u32 = 1610;
    pub const INSTALL_SOURCE_ABSENT: u32 = 1612;
    pub const INSTALL_ALREADY_RUNNING: u32 = 1618;
    pub const PACKAGE_OPEN_FAILED: u32 = 1619;
    pub const BAD_PACKAGE: u32 = 1620;
    pub const FUNCTION_NOT_CALLED: u32 = 1626;
    pub const FUNCTION_FAILED: u32 = 1627;
    pub const INVALID_COMMAND_LINE: u32 = 1639;
    pub const PRODUCT_VERSION: u32 = 1638;
    pub const INSTALL_PLATFORM_UNSUPPORTED: u32 = 1633;
    pub const SUCCESS_REBOOT_INITIATED: u32 = 1641;
    pub const INSUFFICIENT_PRIVILEGES: u32 = 1925;
    pub const SUCCESS_REBOOT_REQUIRED: u32 = 3010;
}

/// Where an installation works: the file system, the registry, services
/// and programs. Paths are Windows paths.
pub trait Target {
    fn create_folder(&mut self, path: &str) -> Result<(), u32>;
    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), u32>;
    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, u32>;
    fn exists(&mut self, path: &str) -> bool;
    fn remove_file(&mut self, path: &str) -> Result<(), u32>;
    /// Remove an empty folder (a folder still holding files stays).
    fn remove_folder(&mut self, path: &str) -> Result<(), u32>;
    fn set_value(
        &mut self,
        root: Root,
        key: &str,
        name: Option<&str>,
        value: &RegValue,
    ) -> Result<(), u32>;
    fn create_key(&mut self, root: Root, key: &str) -> Result<(), u32>;
    fn value(&mut self, root: Root, key: &str, name: &str) -> Option<RegValue>;
    fn delete_value(&mut self, root: Root, key: &str, name: Option<&str>);
    /// Delete a key with its values (and subkeys).
    fn delete_key(&mut self, root: Root, key: &str);
    /// The key's values (name, value); the default value has an empty name.
    fn values(&mut self, root: Root, key: &str) -> Vec<(String, RegValue)> {
        let _ = (root, key);
        Vec::new()
    }
    fn subkeys(&mut self, root: Root, key: &str) -> Vec<String> {
        let _ = (root, key);
        Vec::new()
    }
    /// Append to a file (the rollback script).
    fn append_file(&mut self, path: &str, data: &[u8]) -> Result<(), u32> {
        let mut old = self.read_file(path).unwrap_or_default();
        old.extend_from_slice(data);
        self.write_file(path, &old)
    }
    /// Make a file's bytes durable before the next change.
    fn flush(&mut self, path: &str) {
        let _ = path;
    }
    /// Move a file (to and from the rollback backups).
    fn rename(&mut self, from: &str, to: &str) -> Result<(), u32> {
        let data = self.read_file(from)?;
        self.write_file(to, &data)?;
        self.remove_file(from)
    }
    /// The file names in a folder.
    fn list(&mut self, folder: &str) -> Vec<String> {
        let _ = folder;
        Vec::new()
    }
    /// Free bytes on the volume holding `path`.
    fn free_bytes(&mut self, path: &str) -> Option<u64> {
        let _ = path;
        None
    }
    /// (created, modified) times of a file, in seconds.
    fn file_times(&mut self, path: &str) -> Option<(u64, u64)> {
        let _ = path;
        None
    }
    /// Make a just-installed file's modified time its creation time (so a
    /// later change by the user shows).
    fn stamp(&mut self, path: &str) {
        let _ = path;
    }
    /// Whether the token holds the Administrators group (a per-machine
    /// installation needs it).
    fn elevated(&mut self) -> bool {
        true
    }
    /// The identity of this boot: an installation recorded in progress by
    /// an earlier boot was interrupted.
    fn boot(&mut self) -> u64 {
        0
    }
    /// Whether a running installation holds `path` (its rollback script)
    /// open; None when the target cannot tell (the boot decides).
    fn in_use(&mut self, path: &str) -> Option<bool> {
        let _ = path;
        None
    }
    /// Today as `YYYYMMDD` (InstallDate).
    fn today(&mut self) -> Option<String> {
        None
    }
    /// Run a program (an EXE custom action): its exit code, or a Win32
    /// error when it could not start. `wait` false: started and left.
    fn run(&mut self, command_line: &str, directory: &str, wait: bool) -> Result<u32, u32> {
        let _ = (command_line, directory, wait);
        Err(2)
    }
    /// Call `entry` of the DLL at `path` with an MSIHANDLE for `state` (a
    /// DLL custom action); its result.
    fn call_dll(
        &mut self,
        path: &str,
        entry: &str,
        state: &mut crate::engine::State,
    ) -> Result<u32, u32> {
        let _ = (path, entry, state);
        Err(1157) // ERROR_DLL_NOT_FOUND
    }
    /// Register (`register`) or unregister the type library at `path` with
    /// its help folder `help`, for the machine or the user (`per_user`), as
    /// oleaut32's RegisterTypeLib(ForUser) and UnRegisterTypeLib(ForUser)
    /// do: an HRESULT on failure.
    /// `view32`: the library is a 32-bit component's (registered in the
    /// registry's 32-bit view, as a 32-bit process registers it).
    fn register_typelib(
        &mut self,
        path: &str,
        help: &str,
        per_user: bool,
        view32: bool,
        register: bool,
    ) -> Result<(), u32> {
        let _ = (path, help, per_user, view32, register);
        Err(0x8007_0002)
    }
    /// Call the module's DllRegisterServer (`register`) or
    /// DllUnregisterServer: an HRESULT on failure.
    fn self_register(&mut self, path: &str, register: bool) -> Result<(), u32> {
        let _ = (path, register);
        Err(0x8007_0002)
    }
    /// Tell running programs the environment changed (WM_SETTINGCHANGE
    /// "Environment").
    fn environment_changed(&mut self) {}
    fn install_service(&mut self, service: &ServicePlan) -> Result<(), u32> {
        let _ = service;
        Err(1060)
    }
    fn delete_service(&mut self, name: &str) -> Result<(), u32> {
        let _ = name;
        Err(1060)
    }
    fn start_service(&mut self, name: &str, wait: bool) -> Result<(), u32> {
        let _ = (name, wait);
        Err(1060)
    }
    fn stop_service(&mut self, name: &str, wait: bool) -> Result<(), u32> {
        let _ = (name, wait);
        Err(1060)
    }
    fn service_exists(&mut self, name: &str) -> bool {
        let _ = name;
        false
    }
    /// "Action start" / progress lines for the log.
    fn log(&mut self, line: &str) {
        let _ = line;
    }
}

/// The packed ("squished") GUID form the Installer's registry keys use:
/// each group's hex digits reversed (the last two groups by byte).
pub fn packed_guid(guid: &str) -> Option<String> {
    let hex: Vec<char> = guid
        .trim_matches(|c| c == '{' || c == '}')
        .chars()
        .filter(|c| *c != '-')
        .collect();
    if hex.len() != 32 || !hex.iter().all(char::is_ascii_hexdigit) {
        return None;
    }
    let mut out = String::with_capacity(32);
    for (start, len) in [(0, 8), (8, 4), (12, 4)] {
        out.extend(hex[start..start + len].iter().rev());
    }
    for pair in hex[16..].chunks(2) {
        out.push(pair[1]);
        out.push(pair[0]);
    }
    Some(out.to_ascii_uppercase())
}
/// The braced GUID of a packed one.
pub fn unpacked_guid(packed: &str) -> Option<String> {
    let p: Vec<char> = packed.chars().collect();
    if p.len() != 32 || !p.iter().all(char::is_ascii_hexdigit) {
        return None;
    }
    let mut hex = String::with_capacity(32);
    for (start, len) in [(0, 8), (8, 4), (12, 4)] {
        hex.extend(p[start..start + len].iter().rev());
    }
    for pair in p[16..].chunks(2) {
        hex.push(pair[1]);
        hex.push(pair[0]);
    }
    let hex = hex.to_ascii_uppercase();
    Some(format!(
        "{{{}-{}-{}-{}-{}}}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

/// Split `HKEY_LOCAL_MACHINE\rest` into its root and key.
pub fn split_root(full: &str) -> (Root, &str) {
    let (root, rest) = full.split_once('\\').unwrap_or((full, ""));
    let root = match root {
        "HKEY_CURRENT_USER" => Root::CurrentUser,
        "HKEY_CLASSES_ROOT" => Root::ClassesRoot,
        "HKEY_USERS" => Root::Users,
        _ => Root::LocalMachine,
    };
    (root, rest)
}

/// The Installer's records of installed products, as Windows keeps them
/// (shortened where Doors' registry paths are bounded): per-machine under
/// `HKLM\Software\Classes\Installer`, per-user under
/// `HKCU\Software\Microsoft\Installer`.
pub mod records {
    use super::*;
    pub fn base(all_users: bool) -> (Root, &'static str) {
        if all_users {
            (Root::LocalMachine, "Software\\Classes\\Installer")
        } else {
            (Root::CurrentUser, "Software\\Microsoft\\Installer")
        }
    }
    pub fn product_key(all_users: bool, product: &str) -> Option<(Root, String)> {
        let (root, base) = base(all_users);
        Some((root, format!("{base}\\Products\\{}", packed_guid(product)?)))
    }
    pub fn features_key(all_users: bool, product: &str) -> Option<(Root, String)> {
        let (root, base) = base(all_users);
        Some((root, format!("{base}\\Features\\{}", packed_guid(product)?)))
    }
    pub fn upgrade_key(all_users: bool, upgrade: &str) -> Option<(Root, String)> {
        let (root, base) = base(all_users);
        Some((
            root,
            format!("{base}\\UpgradeCodes\\{}", packed_guid(upgrade)?),
        ))
    }
    /// Component registrations are per-machine (the clients name the
    /// product).
    pub fn component_key(component: &str) -> Option<(Root, String)> {
        Some((
            Root::LocalMachine,
            format!(
                "Software\\Classes\\Installer\\Components\\{}",
                packed_guid(component)?
            ),
        ))
    }
    /// The client a permanent component keeps forever.
    pub const PERMANENT_CLIENT: &str = "00000000000000000000000000000000";
    pub const SHARED_DLLS: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\SharedDLLs";
    /// Longest registry value name the Doors registry stores (UTF-16 units).
    pub const VALUE_NAME_UNITS: usize = 48;
    /// The SharedDLLs value of a file: Windows names it by the file's full
    /// path. A path longer than a Doors value name may be is named instead
    /// by `#` and the FNV-1a hash of its lower-case spelling (16 hex digits).
    pub fn shared_dll_name(path: &str) -> String {
        if path.encode_utf16().count() <= VALUE_NAME_UNITS {
            return String::from(path);
        }
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for unit in path.to_lowercase().encode_utf16() {
            for byte in unit.to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
            }
        }
        format!("#{hash:016x}")
    }

    /// An installed product's record.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Product {
        pub code: String,
        pub all_users: bool,
        pub name: String,
        pub version: String,
        pub publisher: String,
        pub package_code: String,
        pub upgrade_code: String,
        pub local_package: String,
        pub install_source: String,
        pub install_date: String,
        pub language: String,
        /// The installation's directory choices and other remembered
        /// properties (`NAME=value`).
        pub properties: Vec<(String, String)>,
        pub features: Vec<String>,
    }
    fn text(v: Option<RegValue>) -> String {
        match v {
            Some(RegValue::String(s) | RegValue::Expand(s)) => s,
            Some(RegValue::Dword(d)) => format!("{d}"),
            _ => String::new(),
        }
    }
    /// The record of `product` (per-machine first, as Windows looks).
    pub fn product(target: &mut dyn Target, product: &str) -> Option<Product> {
        for all_users in [true, false] {
            let (root, key) = product_key(all_users, product)?;
            let Some(package) = target.value(root, &key, "LocalPackage") else {
                continue;
            };
            let v = |t: &mut dyn Target, n: &str| text(t.value(root, &key, n));
            let properties = match target.value(root, &key, "Properties") {
                Some(RegValue::Multi(m)) => m
                    .iter()
                    .filter_map(|p| p.split_once('='))
                    .map(|(k, v)| (String::from(k), String::from(v)))
                    .collect(),
                _ => Vec::new(),
            };
            let (froot, fkey) = features_key(all_users, product)?;
            let features = target
                .values(froot, &fkey)
                .into_iter()
                .map(|(n, _)| n)
                .filter(|n| !n.is_empty())
                .collect();
            return Some(Product {
                code: String::from(product),
                all_users,
                name: v(target, "ProductName"),
                version: v(target, "VersionString"),
                publisher: v(target, "Publisher"),
                package_code: v(target, "PackageCode"),
                upgrade_code: v(target, "UpgradeCode"),
                local_package: text(Some(package)),
                install_source: v(target, "InstallSource"),
                install_date: v(target, "InstallDate"),
                language: v(target, "Language"),
                properties,
                features,
            });
        }
        None
    }
    /// Every installed product's code (per-machine, then the user's).
    pub fn products(target: &mut dyn Target) -> Vec<String> {
        let mut out = Vec::new();
        for all_users in [true, false] {
            let (root, base) = base(all_users);
            for packed in target.subkeys(root, &format!("{base}\\Products")) {
                if let Some(code) = unpacked_guid(&packed)
                    && !out.contains(&code)
                {
                    out.push(code);
                }
            }
        }
        out
    }
    /// The installed products of an upgrade code.
    pub fn related(target: &mut dyn Target, upgrade: &str) -> Vec<String> {
        let mut out = Vec::new();
        for all_users in [true, false] {
            let Some((root, key)) = upgrade_key(all_users, upgrade) else {
                continue;
            };
            for (packed, _) in target.values(root, &key) {
                if let Some(code) = unpacked_guid(&packed)
                    && !out.contains(&code)
                {
                    out.push(code);
                }
            }
        }
        out
    }
    /// A component's clients (packed product codes) and their key paths.
    pub fn clients(target: &mut dyn Target, component: &str) -> Vec<(String, String)> {
        let Some((root, key)) = component_key(component) else {
            return Vec::new();
        };
        target
            .values(root, &key)
            .into_iter()
            .map(|(n, v)| (n, text(Some(v))))
            .filter(|(n, _)| !n.is_empty())
            .collect()
    }
}

/// The cached package of an installed product, by product code.
pub fn cached_package(product_code: &str, target: &mut dyn Target) -> Option<String> {
    records::product(target, product_code).map(|p| p.local_package)
}
