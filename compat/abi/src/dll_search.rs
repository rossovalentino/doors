//! Where LoadLibrary looks for a DLL named without a folder, as Windows'
//! safe search order has it (Microsoft Learn, "Dynamic-link library search
//! order"): KnownDLLs only in System32; otherwise the application's folder,
//! System32, the Windows folder, then the current directory, or with
//! LoadLibraryEx's LOAD_LIBRARY_SEARCH_* flags only the folders they name.
//! Doors' own system DLLs are found before any of this (the module
//! catalog). Allocation-free; ntdll builds the candidate paths.

#[path = "system32_dlls.rs"]
mod system32;

/// The KnownDLLs of Windows 11 26H2 (HKLM\SYSTEM\CurrentControlSet\Control\
/// Session Manager\KnownDLLs, read on the owner's PC on 11 November 2026),
/// without the WOW64 and emulation modules: always System32's copy.
pub const KNOWN_DLLS: [&str; 26] = [
    "advapi32.dll",
    "clbcatq.dll",
    "combase.dll",
    "comdlg32.dll",
    "coml2.dll",
    "difxapi.dll",
    "gdi32.dll",
    "gdiplus.dll",
    "imagehlp.dll",
    "imm32.dll",
    "kernel32.dll",
    "msctf.dll",
    "msvcrt.dll",
    "normaliz.dll",
    "nsi.dll",
    "ole32.dll",
    "oleaut32.dll",
    "rpcrt4.dll",
    "sechost.dll",
    "setupapi.dll",
    "shcore.dll",
    "shell32.dll",
    "shlwapi.dll",
    "user32.dll",
    "wldap32.dll",
    "ws2_32.dll",
];

/// LoadLibraryEx search flags.
pub mod flags {
    pub const DLL_LOAD_DIR: u32 = 0x100;
    pub const APPLICATION_DIR: u32 = 0x200;
    pub const USER_DIRS: u32 = 0x400;
    pub const SYSTEM32: u32 = 0x800;
    pub const DEFAULT_DIRS: u32 = 0x1000;
    pub const ANY: u32 = DLL_LOAD_DIR | APPLICATION_DIR | USER_DIRS | SYSTEM32 | DEFAULT_DIRS;
}

/// One place to look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The executable's folder.
    Application,
    /// C:\Windows\System32 (SysWOW64 for a 32-bit process, through the
    /// file system redirector).
    System32,
    /// C:\Windows.
    Windows,
    /// The current directory.
    Current,
}

/// FNV-1a over the bytes.
pub const fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u32;
        h = h.wrapping_mul(0x0100_0193);
        i += 1;
    }
    h
}

/// The file name in lower case (ASCII only: another name is no system
/// DLL's), into `out`.
fn lower<'a>(name: &[u16], out: &'a mut [u8; 64]) -> Option<&'a [u8]> {
    if name.len() > out.len() {
        return None;
    }
    for (o, &u) in out.iter_mut().zip(name) {
        *o = u8::try_from(u).ok()?.to_ascii_lowercase();
    }
    Some(&out[..name.len()])
}
/// Whether Windows' System32 holds a DLL of this file name.
pub fn windows_system_dll(name: &[u16]) -> bool {
    let mut buffer = [0u8; 64];
    lower(name, &mut buffer)
        .is_some_and(|n| system32::SYSTEM32_DLLS.binary_search(&fnv1a32(n)).is_ok())
}
/// Whether the file name is one of the KnownDLLs.
pub fn known_dll(name: &[u16]) -> bool {
    let mut buffer = [0u8; 64];
    lower(name, &mut buffer).is_some_and(|n| KNOWN_DLLS.iter().any(|k| k.as_bytes() == n))
}

/// Where to look for the file name `name` (no folder), in order, into
/// `out`: its count.
pub fn places(name: &[u16], search: u32, out: &mut [Place; 4]) -> usize {
    let mut n = 0;
    let mut push = |p: Place| {
        out[n] = p;
        n += 1;
    };
    if known_dll(name) {
        push(Place::System32);
    } else if search & flags::ANY != 0 {
        // Only the folders the flags name; never the current directory.
        if search & (flags::APPLICATION_DIR | flags::DEFAULT_DIRS) != 0 {
            push(Place::Application);
        }
        if search & (flags::SYSTEM32 | flags::DEFAULT_DIRS) != 0 {
            push(Place::System32);
        }
    } else {
        push(Place::Application);
        push(Place::System32);
        push(Place::Windows);
        // A name Windows finds in System32 never comes from here.
        if !windows_system_dll(name) {
            push(Place::Current);
        }
    }
    n
}

/// The file name of `path` with ".dll" when it has no extension (a final
/// "." means none and goes), into `out`: its length, or None if too long.
pub fn with_extension(path: &[u16], out: &mut [u16]) -> Option<usize> {
    let start = path
        .iter()
        .rposition(|&u| u == u16::from(b'\\') || u == u16::from(b'/'))
        .map_or(0, |i| i + 1);
    let file = &path[start..];
    let (body, add) = if file.last() == Some(&u16::from(b'.')) {
        (&path[..path.len() - 1], false)
    } else {
        (path, !file.contains(&u16::from(b'.')))
    };
    let length = body.len() + if add { 4 } else { 0 };
    if length > out.len() {
        return None;
    }
    out[..body.len()].copy_from_slice(body);
    if add {
        for (o, c) in out[body.len()..length].iter_mut().zip(b".dll") {
            *o = u16::from(*c);
        }
    }
    Some(length)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    fn w(s: &str) -> std::vec::Vec<u16> {
        s.encode_utf16().collect()
    }
    #[test]
    fn windows_order_and_planting() {
        let mut out = [Place::Current; 4];
        // An application's own DLL: its folder, System32, Windows, then the
        // current directory.
        let n = places(&w("vendor.dll"), 0, &mut out);
        assert_eq!(
            out[..n],
            [
                Place::Application,
                Place::System32,
                Place::Windows,
                Place::Current
            ]
        );
        // A Windows System32 DLL Doors does not ship: never the current
        // directory.
        assert!(windows_system_dll(&w("VERSION.dll")));
        let n = places(&w("version.dll"), 0, &mut out);
        assert_eq!(
            out[..n],
            [Place::Application, Place::System32, Place::Windows]
        );
        // KnownDLLs: System32 only.
        let n = places(&w("ShLwApi.dll"), 0, &mut out);
        assert_eq!(out[..n], [Place::System32]);
        // LOAD_LIBRARY_SEARCH_SYSTEM32 and DEFAULT_DIRS.
        let n = places(&w("vendor.dll"), flags::SYSTEM32, &mut out);
        assert_eq!(out[..n], [Place::System32]);
        let n = places(&w("vendor.dll"), flags::DEFAULT_DIRS, &mut out);
        assert_eq!(out[..n], [Place::Application, Place::System32]);
        assert_eq!(fnv1a32(b"version.dll"), 0xfc99_c78d);
    }
    #[test]
    fn a_name_without_extension_gets_dll() {
        let mut out = [0u16; 32];
        let n = with_extension(&w("mylib"), &mut out).unwrap();
        assert_eq!(&out[..n], &w("mylib.dll")[..]);
        let n = with_extension(&w("C:\\a.b\\mylib"), &mut out).unwrap();
        assert_eq!(&out[..n], &w("C:\\a.b\\mylib.dll")[..]);
        let n = with_extension(&w("plain."), &mut out).unwrap();
        assert_eq!(&out[..n], &w("plain")[..]);
        let n = with_extension(&w("x.drv"), &mut out).unwrap();
        assert_eq!(&out[..n], &w("x.drv")[..]);
    }
}
