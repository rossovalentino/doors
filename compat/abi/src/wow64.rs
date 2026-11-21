//! WOW64's views (P17), as Windows gives them to 32-bit programs: the
//! registry's 32-bit view of HKLM\SOFTWARE (the WOW6432Node subkey, except
//! the keys Windows shares between the views, and the redirected COM
//! subkeys of Software\Classes), the file system redirector
//! (C:\Windows\System32 is C:\Windows\SysWOW64 for a 32-bit program, except
//! the folders Windows exempts; C:\Windows\Sysnative is the real System32),
//! and the environment of a 32-bit process (and of a 64-bit process a
//! 32-bit one started). Allocation-free functions over caller buffers;
//! ntdll and advapi32 apply them. Sources: Microsoft Learn, "Registry Keys
//! Affected by WOW64", "File System Redirector"; the environment and views
//! measured on Windows 11 26H2 (`tests/fixtures/windows-x86/wowprobe.c`).

/// The registry views (KEY_WOW64_64KEY, KEY_WOW64_32KEY).
pub const KEY_WOW64_64KEY: u32 = 0x100;
pub const KEY_WOW64_32KEY: u32 = 0x200;
pub const NODE: &str = "WOW6432Node";

/// Whether a key opened with `desired` by a 32-bit (`wow64`) or 64-bit
/// process is in the 32-bit view: the explicit flag wins, else the
/// process's own view.
pub const fn view32(desired: u32, wow64: bool) -> bool {
    if desired & KEY_WOW64_64KEY != 0 {
        false
    } else if desired & KEY_WOW64_32KEY != 0 {
        true
    } else {
        wow64
    }
}

/// The keys under HKLM\SOFTWARE that both views share (not redirected).
const SHARED: &[&str] = &[
    "Clients",
    "Microsoft\\COM3",
    "Microsoft\\Cryptography\\Calais\\Current",
    "Microsoft\\Cryptography\\Calais\\Readers",
    "Microsoft\\Cryptography\\Services",
    "Microsoft\\CTF\\SystemShared",
    "Microsoft\\CTF\\TIP",
    "Microsoft\\DFS",
    "Microsoft\\Driver Signing",
    "Microsoft\\EnterpriseCertificates",
    "Microsoft\\EventSystem",
    "Microsoft\\MSMQ",
    "Microsoft\\Non-Driver Signing",
    "Microsoft\\Notepad\\DefaultFonts",
    "Microsoft\\OLE",
    "Microsoft\\RAS",
    "Microsoft\\RPC",
    "Microsoft\\Shared Tools\\MSInfo",
    "Microsoft\\SystemCertificates",
    "Microsoft\\TermServLicensing",
    "Microsoft\\Transaction Server",
    "Microsoft\\Windows\\CurrentVersion\\App Paths",
    "Microsoft\\Windows\\CurrentVersion\\Control Panel\\Cursors\\Schemes",
    "Microsoft\\Windows\\CurrentVersion\\Explorer\\AutoplayHandlers",
    "Microsoft\\Windows\\CurrentVersion\\Explorer\\DriveIcons",
    "Microsoft\\Windows\\CurrentVersion\\Explorer\\KindMap",
    "Microsoft\\Windows\\CurrentVersion\\Group Policy",
    "Microsoft\\Windows\\CurrentVersion\\Policies",
    "Microsoft\\Windows\\CurrentVersion\\PreviewHandlers",
    "Microsoft\\Windows\\CurrentVersion\\Setup",
    "Microsoft\\Windows\\CurrentVersion\\Telephony",
    "Microsoft\\Windows NT\\CurrentVersion\\Console",
    "Microsoft\\Windows NT\\CurrentVersion\\FontDpi",
    "Microsoft\\Windows NT\\CurrentVersion\\FontLink",
    "Microsoft\\Windows NT\\CurrentVersion\\FontMapper",
    "Microsoft\\Windows NT\\CurrentVersion\\Fonts",
    "Microsoft\\Windows NT\\CurrentVersion\\FontSubstitutes",
    "Microsoft\\Windows NT\\CurrentVersion\\Gre_Initialize",
    "Microsoft\\Windows NT\\CurrentVersion\\Image File Execution Options",
    "Microsoft\\Windows NT\\CurrentVersion\\LanguagePack",
    "Microsoft\\Windows NT\\CurrentVersion\\NetworkCards",
    "Microsoft\\Windows NT\\CurrentVersion\\Perflib",
    "Microsoft\\Windows NT\\CurrentVersion\\Ports",
    "Microsoft\\Windows NT\\CurrentVersion\\Print",
    "Microsoft\\Windows NT\\CurrentVersion\\ProfileList",
    "Microsoft\\Windows NT\\CurrentVersion\\Time Zones",
    "Policies",
    "RegisteredApplications",
];
/// The Software\Classes subkeys the 32-bit view redirects (to
/// Classes\WOW6432Node); every other class key is shared.
const CLASSES_REDIRECTED: &[&str] = &[
    "CLSID",
    "DirectShow",
    "Interface",
    "Media Type",
    "MediaFoundation",
];

const BACKSLASH: u16 = b'\\' as u16;
fn fold(unit: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&unit) {
        unit - 32
    } else {
        unit
    }
}
/// `a` equals the ASCII text `b`, ignoring case.
fn same(a: &[u16], b: &str) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.bytes())
            .all(|(x, y)| fold(*x) == fold(u16::from(y)))
}
/// The non-empty components of a key path.
fn parts(path: &[u16]) -> impl Iterator<Item = &[u16]> + Clone {
    path.split(|u| *u == BACKSLASH).filter(|p| !p.is_empty())
}
/// Whether `path`'s components begin with those of `prefix`.
fn starts<'a>(mut path: impl Iterator<Item = &'a [u16]>, prefix: &str) -> bool {
    prefix
        .split('\\')
        .all(|wanted| path.next().is_some_and(|part| same(part, wanted)))
}
/// Appends components to `out` (backslash-separated); None when it is full.
struct Writer<'a> {
    out: &'a mut [u16],
    length: usize,
}
impl Writer<'_> {
    fn push(&mut self, part: &[u16]) -> Option<()> {
        let separator = usize::from(self.length != 0);
        let end = self.length.checked_add(separator + part.len())?;
        let slot = self.out.get_mut(self.length..end)?;
        if separator == 1 {
            slot[0] = BACKSLASH;
        }
        slot[separator..].copy_from_slice(part);
        self.length = end;
        Some(())
    }
    fn push_text(&mut self, part: &str) -> Option<()> {
        let mut units = [0u16; 16];
        for (slot, byte) in units.iter_mut().zip(part.bytes()) {
            *slot = u16::from(byte);
        }
        self.push(&units[..part.len()])
    }
}

/// Where a key of the 32-bit view lives in the one registry tree, into
/// `out`: its length, or None when `out` is too small. `root` is storage's
/// hive (0 HKLM, 1 HKCU, 2 HKCR); `path` is the key's path below it as the
/// program named it. HKLM\SOFTWARE\X becomes SOFTWARE\WOW6432Node\X unless
/// X is shared or already the node; a redirected class key (HKLM or HKCU
/// Software\Classes\X, or HKCR\X) moves below Classes\WOW6432Node. (The
/// 64-bit view is the path itself.)
pub fn registry_path(root: u8, path: &[u16], out: &mut [u16]) -> Option<usize> {
    let mut w = Writer { out, length: 0 };
    let mut all = parts(path);
    let first = all.clone().next();
    let second = all.clone().nth(1);
    let software = first.is_some_and(|p| same(p, "SOFTWARE"));
    // Where a class key's name is: HKCR's first component, or the third of
    // HKLM/HKCU Software\Classes.
    let classes_at = match root {
        2 => Some(0),
        0 | 1 if software && second.is_some_and(|p| same(p, "Classes")) => Some(2),
        _ => None,
    };
    if let Some(at) = classes_at {
        let redirected = all
            .clone()
            .nth(at)
            .is_some_and(|name| CLASSES_REDIRECTED.iter().any(|c| same(name, c)));
        for (index, part) in all.enumerate() {
            if index == at && redirected {
                w.push_text(NODE)?;
            }
            w.push(part)?;
        }
        return Some(w.length);
    }
    let rest = all.clone().skip(1);
    let redirected = root == 0
        && software
        && !second.is_some_and(|p| same(p, NODE))
        && !SHARED.iter().any(|shared| starts(rest.clone(), shared));
    if let Some(head) = all.next() {
        w.push(head)?;
        if redirected {
            w.push_text(NODE)?;
        }
    }
    for part in all {
        w.push(part)?;
    }
    Some(w.length)
}

/// The folders under System32 the redirector leaves alone.
const EXEMPT: &[&str] = &[
    "catroot",
    "catroot2",
    "driverstore",
    "drivers\\etc",
    "logfiles",
    "spool",
];
const SYSTEM32: &str = "c:\\windows\\system32";
const SYSNATIVE: &str = "c:\\windows\\sysnative";
/// `path` begins with `prefix` (ASCII case ignored) followed by its end
/// or a backslash: what follows.
fn under<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let head = path.get(..prefix.len())?;
    let rest = &path[prefix.len()..];
    (head.eq_ignore_ascii_case(prefix) && (rest.is_empty() || rest.starts_with('\\')))
        .then_some(rest)
}
/// A normalized Windows path (`C:\...`) as a 32-bit program's file system
/// access resolves it: C:\Windows\Sysnative\... is always the real
/// C:\Windows\System32\...; with `redirect` (the thread's redirection is
/// on) C:\Windows\System32\... is C:\Windows\SysWOW64\... except the
/// exempt folders. Ok(None): unchanged; Ok(Some(n)): the path is
/// `out[..n]`; Err(Size): `out` is too small.
pub fn file_path(
    path: &str,
    redirect: bool,
    out: &mut [u8],
) -> Result<Option<usize>, super::Error> {
    let mut put = |head: &str, rest: &str| -> Result<Option<usize>, super::Error> {
        let n = head.len() + rest.len();
        let slot = out.get_mut(..n).ok_or(super::Error::Size)?;
        slot[..head.len()].copy_from_slice(head.as_bytes());
        slot[head.len()..].copy_from_slice(rest.as_bytes());
        Ok(Some(n))
    };
    if let Some(rest) = under(path, SYSNATIVE) {
        return put("C:\\Windows\\System32", rest);
    }
    if !redirect {
        return Ok(None);
    }
    let Some(rest) = under(path, SYSTEM32) else {
        return Ok(None);
    };
    let inner = rest.trim_start_matches('\\');
    if EXEMPT.iter().any(|e| under(inner, e).is_some()) {
        return Ok(None);
    }
    put("C:\\Windows\\SysWOW64", rest)
}

/// A variable's value copied out (at most 260 units), or None.
fn value(block: &super::environment::Block, name: &str, out: &mut [u16; 260]) -> Option<usize> {
    let mut wide = [0u16; 32];
    for (slot, byte) in wide.iter_mut().zip(name.bytes()) {
        *slot = u16::from(byte);
    }
    let v = block.get(&wide[..name.len()])?;
    let n = v.len().min(out.len());
    out[..n].copy_from_slice(&v[..n]);
    Some(n)
}
fn set(
    block: &mut super::environment::Block,
    name: &str,
    value: Option<&[u16]>,
) -> Result<(), super::Error> {
    let mut wide = [0u16; 32];
    for (slot, byte) in wide.iter_mut().zip(name.bytes()) {
        *slot = u16::from(byte);
    }
    block.set(&wide[..name.len()], value)
}
fn text(s: &str, out: &mut [u16; 260]) -> usize {
    for (slot, byte) in out.iter_mut().zip(s.bytes()) {
        *slot = u16::from(byte);
    }
    s.len()
}

/// The environment variables WOW64 changes, as a 32-bit process sees them
/// (`x86`), or as a 64-bit process a 32-bit parent started sees them (the
/// 64-bit values restored; measured: wowchild.exe report exits 31). A
/// 64-bit parent's block passes to a 64-bit child unchanged.
pub fn environment(block: &mut super::environment::Block, x86: bool) -> Result<(), super::Error> {
    let mut a = [0u16; 260];
    let mut b = [0u16; 260];
    let mut arch = [0u16; 260];
    let native = match value(block, "PROCESSOR_ARCHITEW6432", &mut arch) {
        Some(n) => Some(n),
        None => match value(block, "PROCESSOR_ARCHITECTURE", &mut arch) {
            Some(n) if !same(&arch[..n], "x86") => Some(n),
            _ => None,
        },
    };
    let arch_n = native.unwrap_or_else(|| text("AMD64", &mut arch));
    if x86 {
        let files = value(block, "ProgramFiles(x86)", &mut a)
            .unwrap_or_else(|| text("C:\\Program Files (x86)", &mut a));
        let common = value(block, "CommonProgramFiles(x86)", &mut b)
            .unwrap_or_else(|| text("C:\\Program Files (x86)\\Common Files", &mut b));
        set(block, "PROCESSOR_ARCHITECTURE", Some(&[120, 56, 54]))?;
        set(block, "PROCESSOR_ARCHITEW6432", Some(&arch[..arch_n]))?;
        set(block, "ProgramFiles", Some(&a[..files]))?;
        set(block, "CommonProgramFiles", Some(&b[..common]))?;
    } else if value(block, "PROCESSOR_ARCHITEW6432", &mut a).is_some() {
        let files = value(block, "ProgramW6432", &mut a)
            .unwrap_or_else(|| text("C:\\Program Files", &mut a));
        let common = value(block, "CommonProgramW6432", &mut b)
            .unwrap_or_else(|| text("C:\\Program Files\\Common Files", &mut b));
        set(block, "PROCESSOR_ARCHITECTURE", Some(&arch[..arch_n]))?;
        set(block, "PROCESSOR_ARCHITEW6432", None)?;
        set(block, "ProgramFiles", Some(&a[..files]))?;
        set(block, "CommonProgramFiles", Some(&b[..common]))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{boxed::Box, string::String, vec::Vec};
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn view(root: u8, path: &str) -> String {
        let mut out = [0u16; 256];
        let n = registry_path(root, &w(path), &mut out).unwrap();
        String::from_utf16(&out[..n]).unwrap()
    }
    fn file(path: &str, redirect: bool) -> Option<String> {
        let mut out = [0u8; 260];
        file_path(path, redirect, &mut out)
            .unwrap()
            .map(|n| String::from_utf8(out[..n].to_vec()).unwrap())
    }
    #[test]
    fn the_32_bit_registry_view_redirects_software_but_shares_what_windows_shares() {
        assert_eq!(view(0, "SOFTWARE"), "SOFTWARE\\WOW6432Node");
        assert_eq!(
            view(0, "software\\Microsoft\\Windows\\CurrentVersion"),
            "software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion"
        );
        assert_eq!(
            view(
                0,
                "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\x.exe"
            ),
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\x.exe"
        );
        assert_eq!(view(0, "SOFTWARE\\Policies"), "SOFTWARE\\Policies");
        assert_eq!(
            view(0, "SOFTWARE\\PoliciesX"),
            "SOFTWARE\\WOW6432Node\\PoliciesX"
        );
        assert_eq!(
            view(0, "SOFTWARE\\WOW6432Node\\A"),
            "SOFTWARE\\WOW6432Node\\A"
        );
        assert_eq!(view(0, "SYSTEM\\Setup"), "SYSTEM\\Setup");
        assert_eq!(
            view(0, "SOFTWARE\\Classes\\CLSID\\{1}"),
            "SOFTWARE\\Classes\\WOW6432Node\\CLSID\\{1}"
        );
        assert_eq!(
            view(0, "SOFTWARE\\Classes\\.txt"),
            "SOFTWARE\\Classes\\.txt"
        );
        assert_eq!(
            view(1, "Software\\Classes\\Interface"),
            "Software\\Classes\\WOW6432Node\\Interface"
        );
        assert_eq!(view(2, "CLSID\\{1}"), "WOW6432Node\\CLSID\\{1}");
        assert_eq!(view(1, "Software\\Doors"), "Software\\Doors");
        assert_eq!(view(0, ""), "");
        assert!(registry_path(0, &w("SOFTWARE\\A"), &mut [0u16; 8]).is_none());
        assert!(view32(0, true) && !view32(KEY_WOW64_64KEY, true));
        assert!(view32(KEY_WOW64_32KEY, false) && !view32(0, false));
    }
    #[test]
    fn system32_is_syswow64_except_the_exempt_folders() {
        assert_eq!(
            file("C:\\Windows\\System32\\x.dll", true).as_deref(),
            Some("C:\\Windows\\SysWOW64\\x.dll")
        );
        assert_eq!(
            file("C:\\WINDOWS\\system32", true).as_deref(),
            Some("C:\\Windows\\SysWOW64")
        );
        assert_eq!(
            file("C:\\Windows\\System32\\drivers\\etc\\hosts", true),
            None
        );
        assert_eq!(file("C:\\Windows\\System32\\CatRoot\\{x}", true), None);
        assert_eq!(
            file("C:\\Windows\\System32\\drivers\\x.sys", true).as_deref(),
            Some("C:\\Windows\\SysWOW64\\drivers\\x.sys")
        );
        assert_eq!(file("C:\\Windows\\System32\\x.dll", false), None);
        assert_eq!(file("C:\\Windows\\System32x\\a", true), None);
        assert_eq!(
            file("C:\\Windows\\Sysnative\\x.dll", false).as_deref(),
            Some("C:\\Windows\\System32\\x.dll")
        );
        assert!(file_path("C:\\Windows\\System32\\x", true, &mut [0u8; 8]).is_err());
    }
    #[test]
    fn environments_follow_the_process_width() {
        let mut block = Box::new(crate::environment::Block::new());
        crate::environment::session_defaults(&mut block, "u", "C:\\Users\\u", "PC", "4").unwrap();
        let get = |block: &crate::environment::Block, name: &str| {
            block.get(&w(name)).map(|v| String::from_utf16(v).unwrap())
        };
        // A 64-bit parent's block is left alone.
        environment(&mut block, false).unwrap();
        assert_eq!(
            get(&block, "ProgramFiles").as_deref(),
            Some("C:\\Program Files")
        );
        environment(&mut block, true).unwrap();
        assert_eq!(
            get(&block, "ProgramFiles").as_deref(),
            Some("C:\\Program Files (x86)")
        );
        assert_eq!(
            get(&block, "PROCESSOR_ARCHITECTURE").as_deref(),
            Some("x86")
        );
        assert_eq!(
            get(&block, "PROCESSOR_ARCHITEW6432").as_deref(),
            Some("AMD64")
        );
        assert_eq!(
            get(&block, "CommonProgramFiles").as_deref(),
            Some("C:\\Program Files (x86)\\Common Files")
        );
        environment(&mut block, false).unwrap();
        assert_eq!(
            get(&block, "ProgramFiles").as_deref(),
            Some("C:\\Program Files")
        );
        assert_eq!(
            get(&block, "PROCESSOR_ARCHITECTURE").as_deref(),
            Some("AMD64")
        );
        assert!(get(&block, "PROCESSOR_ARCHITEW6432").is_none());
        assert_eq!(
            get(&block, "CommonProgramFiles").as_deref(),
            Some("C:\\Program Files\\Common Files")
        );
    }
}
