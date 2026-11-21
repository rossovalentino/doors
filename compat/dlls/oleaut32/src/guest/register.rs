//! Type-library registration (HKCR\TypeLib\{libid}\maj.min, and
//! HKCR\Interface\{iid} for each automation interface with the standard
//! OLE Automation proxy), lookup for LoadRegTypeLib and
//! QueryPathOfRegTypeLib, and removal. RegisterTypeLibForUser writes the
//! same under HKCU\Software\Classes.
use super::typeinfo;
use super::*;
use crate::typelib::{Library, tkind, typeflag};
use alloc::string::String;

#[link(name = "advapi32", kind = "dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(key: usize, sub: usize, options: u32, desired: u32, out: usize) -> u32;
    fn RegCreateKeyExW(
        key: usize,
        sub: usize,
        reserved: u32,
        class: usize,
        options: u32,
        desired: u32,
        security: usize,
        out: usize,
        disposition: usize,
    ) -> u32;
    fn RegQueryValueExW(
        key: usize,
        name: usize,
        reserved: usize,
        kind: usize,
        data: usize,
        size: usize,
    ) -> u32;
    fn RegSetValueExW(
        key: usize,
        name: usize,
        reserved: u32,
        kind: u32,
        data: usize,
        size: u32,
    ) -> u32;
    fn RegEnumKeyExW(
        key: usize,
        index: u32,
        out: usize,
        length: usize,
        reserved: usize,
        class: usize,
        class_len: usize,
        time: usize,
    ) -> u32;
    fn RegDeleteTreeW(key: usize, sub: usize) -> u32;
    fn RegDeleteKeyW(key: usize, sub: usize) -> u32;
    fn RegCloseKey(key: usize) -> u32;
}
const HKEY_CLASSES_ROOT: u64 = 0x8000_0000;
const HKEY_CURRENT_USER: u64 = 0x8000_0001;
const KEY_READ: u32 = 0x2_0019;
const KEY_ALL_ACCESS: u32 = 0xF_003F;
const REG_SZ: u32 = 1;
const TYPE_E_REGISTRYACCESS: u32 = 0x8002_801C;
/// PSOAInterface and PSDispatch: the proxies for automation interfaces.
const PS_OA_INTERFACE: &str = "{00020424-0000-0000-C000-000000000046}";
const PS_DISPATCH: &str = "{00020420-0000-0000-C000-000000000046}";

fn z(s: &str) -> Vec<u16> {
    let mut w = typeinfo::wide(s);
    w.push(0);
    w
}
/// {XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}.
pub(super) fn guid_text(g: &[u8; 16]) -> String {
    let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    let mut s = alloc::format!("{{{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-", g[8], g[9]);
    for b in &g[10..] {
        s.push_str(&alloc::format!("{b:02X}"));
    }
    s.push('}');
    s
}
struct Key(u64);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(door_windows_abi::narrow(self.0)) };
    }
}
fn open(root: u64, path: &str) -> Option<Key> {
    let mut k = 0u64;
    let p = z(path);
    (unsafe {
        RegOpenKeyExW(
            door_windows_abi::narrow(root),
            door_windows_abi::narrow(p.as_ptr() as u64),
            0,
            KEY_READ,
            door_windows_abi::narrow(&mut k as *mut u64 as u64),
        )
    } == 0)
        .then_some(Key(k))
}
fn create(root: u64, path: &str) -> Result<Key, u32> {
    let mut k = 0u64;
    let p = z(path);
    let r = unsafe {
        RegCreateKeyExW(
            door_windows_abi::narrow(root),
            door_windows_abi::narrow(p.as_ptr() as u64),
            0,
            0,
            0,
            KEY_ALL_ACCESS,
            0,
            door_windows_abi::narrow(&mut k as *mut u64 as u64),
            0,
        )
    };
    if r == 0 {
        Ok(Key(k))
    } else {
        Err(TYPE_E_REGISTRYACCESS)
    }
}
fn set(key: &Key, name: Option<&str>, value: &str) -> Result<(), u32> {
    let v = z(value);
    let n = name.map(z);
    let r = unsafe {
        RegSetValueExW(
            door_windows_abi::narrow(key.0),
            n.as_ref()
                .map_or(0, |n| door_windows_abi::narrow(n.as_ptr() as u64)),
            0,
            REG_SZ,
            door_windows_abi::narrow(v.as_ptr() as u64),
            (v.len() * 2) as u32,
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(TYPE_E_REGISTRYACCESS)
    }
}
fn default_value(key: &Key) -> Option<Vec<u16>> {
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let mut kind = 0u32;
    let r = unsafe {
        RegQueryValueExW(
            door_windows_abi::narrow(key.0),
            0,
            0,
            door_windows_abi::narrow(&mut kind as *mut u32 as u64),
            door_windows_abi::narrow(buffer.as_mut_ptr() as u64),
            door_windows_abi::narrow(&mut size as *mut u32 as u64),
        )
    };
    if r != 0 || (kind != REG_SZ && kind != 2) {
        return None;
    }
    buffer.truncate(size as usize / 2);
    while buffer.last() == Some(&0) {
        buffer.pop();
    }
    Some(buffer)
}
fn subkeys(key: &Key) -> Vec<String> {
    let mut out = Vec::new();
    for index in 0..4096 {
        let mut name = [0u16; 256];
        let mut len = name.len() as u32;
        let r = unsafe {
            RegEnumKeyExW(
                door_windows_abi::narrow(key.0),
                index,
                door_windows_abi::narrow(name.as_mut_ptr() as u64),
                door_windows_abi::narrow(&mut len as *mut u32 as u64),
                0,
                0,
                0,
                0,
            )
        };
        if r != 0 {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    out
}
fn hex_u16(s: &str) -> Option<u16> {
    u16::from_str_radix(s, 16).ok()
}
/// The registered path of library `guid` version `major`.`minor` (the
/// highest minor at or above it) for `lcid` (then its primary language,
/// then neutral), win64 before win32.
pub(super) fn registered_path(
    guid: &[u8; 16],
    major: u16,
    minor: u16,
    lcid: u32,
) -> Result<Vec<u16>, u32> {
    // HKCR as Windows merges it: the user's classes first.
    let base = alloc::format!("TypeLib\\{}", guid_text(guid));
    let lib = open(
        HKEY_CURRENT_USER,
        &alloc::format!("Software\\Classes\\{base}"),
    )
    .or_else(|| open(HKEY_CLASSES_ROOT, &base))
    .ok_or(hr::TYPE_E_LIBNOTREGISTERED)?;
    let version = subkeys(&lib)
        .into_iter()
        .filter_map(|v| {
            let (a, b) = v.split_once('.')?;
            Some((hex_u16(a)?, hex_u16(b)?, v))
        })
        .filter(|(a, b, _)| *a == major && *b >= minor)
        .max_by_key(|(_, b, _)| *b)
        .ok_or(hr::TYPE_E_LIBNOTREGISTERED)?
        .2;
    for l in [lcid, lcid & 0x3ff, 0] {
        for platform in ["win64", "win32"] {
            if let Some(k) = open(lib.0, &alloc::format!("{version}\\{l:x}\\{platform}"))
                && let Some(path) = default_value(&k)
            {
                return Ok(path);
            }
        }
    }
    Err(hr::TYPE_E_LIBNOTREGISTERED)
}
/// `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` as GUID bytes.
pub(super) fn parse_guid(text: &[u16]) -> Option<[u8; 16]> {
    let s: String = String::from_utf16(text).ok()?;
    let s = s.trim().strip_prefix('{')?.strip_suffix('}')?;
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 || parts.iter().map(|p| p.len()).ne([8, 4, 4, 4, 12]) {
        return None;
    }
    let d1 = u32::from_str_radix(parts[0], 16).ok()?;
    let d2 = u16::from_str_radix(parts[1], 16).ok()?;
    let d3 = u16::from_str_radix(parts[2], 16).ok()?;
    let mut g = [0u8; 16];
    g[..4].copy_from_slice(&d1.to_le_bytes());
    g[4..6].copy_from_slice(&d2.to_le_bytes());
    g[6..8].copy_from_slice(&d3.to_le_bytes());
    let tail: String = [parts[3], parts[4]].concat();
    for i in 0..8 {
        g[8 + i] = u8::from_str_radix(&tail[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(g)
}
/// HKCR\`key` (the user's classes first): the type library an Interface
/// key names and its "Version".
pub(super) fn interface_typelib(key: &str) -> Option<([u8; 16], String)> {
    let k = open(
        HKEY_CURRENT_USER,
        &alloc::format!("Software\\Classes\\{key}"),
    )
    .or_else(|| open(HKEY_CLASSES_ROOT, key))?;
    let libid = parse_guid(&default_value(&k)?)?;
    let mut buffer = vec![0u16; 64];
    let mut size = (buffer.len() * 2) as u32;
    let mut kind = 0u32;
    let name = z("Version");
    let version = if unsafe {
        RegQueryValueExW(
            door_windows_abi::narrow(k.0),
            door_windows_abi::narrow(name.as_ptr() as u64),
            0,
            door_windows_abi::narrow(&mut kind as *mut u32 as u64),
            door_windows_abi::narrow(buffer.as_mut_ptr() as u64),
            door_windows_abi::narrow(&mut size as *mut u32 as u64),
        )
    } == 0
    {
        buffer.truncate(size as usize / 2);
        while buffer.last() == Some(&0) {
            buffer.pop();
        }
        String::from_utf16_lossy(&buffer)
    } else {
        String::from("1.0")
    };
    Some((libid, version))
}
fn library_of(ptlib: u64) -> Result<&'static Library, u32> {
    typeinfo::lib_of(ptlib)
        .and_then(typeinfo::library)
        .ok_or(hr::E_INVALIDARG)
}
fn register(root: u64, prefix: &str, ptlib: u64, path: u64, help: u64) -> Result<(), u32> {
    let lib = library_of(ptlib)?;
    let path = text_of(read_wide(path)?);
    let help = if help == 0 {
        match path.rfind('\\') {
            Some(i) => String::from(&path[..i]),
            None => String::new(),
        }
    } else {
        text_of(read_wide(help)?)
    };
    let libid = guid_text(&lib.guid);
    let version = alloc::format!("{:x}.{:x}", lib.major, lib.minor);
    let base = alloc::format!("{prefix}TypeLib\\{libid}\\{version}");
    let key = create(root, &base)?;
    set(&key, None, lib.doc.as_deref().unwrap_or(&lib.name))?;
    set(
        &create(root, &alloc::format!("{base}\\FLAGS"))?,
        None,
        &alloc::format!("{}", lib.flags & !crate::typelib::HAS_DISK_IMAGE),
    )?;
    set(
        &create(root, &alloc::format!("{base}\\HELPDIR"))?,
        None,
        &help,
    )?;
    let platform = if lib.syskind == crate::typelib::SYS_WIN32 {
        "win32"
    } else {
        "win64"
    };
    set(
        &create(root, &alloc::format!("{base}\\{:x}\\{platform}", lib.lcid))?,
        None,
        &path,
    )?;
    for t in &lib.types {
        let automation = t.kind == tkind::DISPATCH
            || (t.kind == tkind::INTERFACE
                && t.flags & (typeflag::OLEAUTOMATION | typeflag::DUAL) != 0);
        if !automation {
            continue;
        }
        let proxy = if t.kind == tkind::DISPATCH && t.flags & typeflag::DUAL == 0 {
            PS_DISPATCH
        } else {
            PS_OA_INTERFACE
        };
        let iface = alloc::format!("{prefix}Interface\\{}", guid_text(&t.guid));
        set(&create(root, &iface)?, None, &t.name)?;
        set(
            &create(root, &alloc::format!("{iface}\\ProxyStubClsid"))?,
            None,
            proxy,
        )?;
        set(
            &create(root, &alloc::format!("{iface}\\ProxyStubClsid32"))?,
            None,
            proxy,
        )?;
        let k = create(root, &alloc::format!("{iface}\\TypeLib"))?;
        set(&k, None, &libid)?;
        set(
            &k,
            Some("Version"),
            &alloc::format!("{}.{}", lib.major, lib.minor),
        )?;
    }
    Ok(())
}
fn text_of(units: Vec<u16>) -> String {
    String::from_utf16_lossy(&units)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterTypeLib(ptlib: usize, path: usize, help: usize) -> u32 {
    let ptlib = door_windows_abi::widen(ptlib);
    let path = door_windows_abi::widen(path);
    let help = door_windows_abi::widen(help);
    result(register(HKEY_CLASSES_ROOT, "", ptlib, path, help))
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterTypeLibForUser(ptlib: usize, path: usize, help: usize) -> u32 {
    let ptlib = door_windows_abi::widen(ptlib);
    let path = door_windows_abi::widen(path);
    let help = door_windows_abi::widen(help);
    {
        result(register(
            HKEY_CURRENT_USER,
            "Software\\Classes\\",
            ptlib,
            path,
            help,
        ))
    }
}
fn unregister(root: u64, prefix: &str, guid: u64, major: u16, minor: u16) -> Result<(), u32> {
    let g = read_n::<16>(guid)?;
    let libid = guid_text(&g);
    let base = alloc::format!("{prefix}TypeLib\\{libid}");
    let version = alloc::format!("{major:x}.{minor:x}");
    if open(root, &alloc::format!("{base}\\{version}")).is_none() {
        return Err(TYPE_E_REGISTRYACCESS);
    }
    // The interfaces it registered (those whose TypeLib names it).
    if let Ok(path) = registered_path(&g, major, minor, 0)
        && let Ok(id) = typeinfo::load(&path)
        && let Some(lib) = typeinfo::library(id)
    {
        for t in &lib.types {
            let iface = alloc::format!("{prefix}Interface\\{}", guid_text(&t.guid));
            let ours = open(root, &alloc::format!("{iface}\\TypeLib"))
                .and_then(|k| default_value(&k))
                .is_some_and(|v| text_of(v).eq_ignore_ascii_case(&libid));
            if ours {
                let p = z(&iface);
                unsafe {
                    RegDeleteTreeW(
                        door_windows_abi::narrow(root),
                        door_windows_abi::narrow(p.as_ptr() as u64),
                    )
                };
                unsafe {
                    RegDeleteKeyW(
                        door_windows_abi::narrow(root),
                        door_windows_abi::narrow(p.as_ptr() as u64),
                    )
                };
            }
        }
    }
    let p = z(&alloc::format!("{base}\\{version}"));
    unsafe {
        RegDeleteTreeW(
            door_windows_abi::narrow(root),
            door_windows_abi::narrow(p.as_ptr() as u64),
        )
    };
    unsafe {
        RegDeleteKeyW(
            door_windows_abi::narrow(root),
            door_windows_abi::narrow(p.as_ptr() as u64),
        )
    };
    if open(root, &base).is_some_and(|k| subkeys(&k).is_empty()) {
        let p = z(&base);
        unsafe {
            RegDeleteKeyW(
                door_windows_abi::narrow(root),
                door_windows_abi::narrow(p.as_ptr() as u64),
            )
        };
    }
    Ok(())
}
#[unsafe(no_mangle)]
pub extern "system" fn UnRegisterTypeLib(
    guid: usize,
    major: u16,
    minor: u16,
    _lcid: u32,
    _sys: u32,
) -> u32 {
    let guid = door_windows_abi::widen(guid);
    result(unregister(HKEY_CLASSES_ROOT, "", guid, major, minor))
}
#[unsafe(no_mangle)]
pub extern "system" fn UnRegisterTypeLibForUser(
    guid: usize,
    major: u16,
    minor: u16,
    _lcid: u32,
    _sys: u32,
) -> u32 {
    let guid = door_windows_abi::widen(guid);
    {
        result(unregister(
            HKEY_CURRENT_USER,
            "Software\\Classes\\",
            guid,
            major,
            minor,
        ))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryPathOfRegTypeLib(
    guid: usize,
    major: u16,
    minor: u16,
    lcid: u32,
    out: usize,
) -> u32 {
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    {
        let r = read_n::<16>(guid)
            .and_then(|g| registered_path(&g, major, minor, lcid))
            .and_then(|p| write_pointer(out, bstr_from(&p)));
        result(r)
    }
}
