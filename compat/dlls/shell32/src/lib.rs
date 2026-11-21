#![no_std]
//! Original shell32.dll: command-line splitting, known folders (FOLDERID and
//! CSIDL) and their item ID lists, ShellExecute(Ex) with registry
//! associations and the runas verb (User Account Control), SHFileOperation,
//! SHGetFileInfo, SHCreateDirectoryEx, IsUserAnAdmin and the HDROP
//! functions.
//!
//! This module is the host-testable part: the folder tables, the item ID
//! list format, double-NUL path lists and file type names.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;
use alloc::{format, string::String, vec::Vec};

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

pub type Guid = [u8; 16];
pub const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> Guid {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}

/// Where a known folder lives: an environment variable's directory plus a
/// relative path, or a fixed path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Profile,
    RoamingAppData,
    LocalAppData,
    Fixed,
}
/// One known folder: its FOLDERID, its CSIDL (if it has one) and where it
/// lives, as a default Windows 11 installation lays them out.
#[derive(Clone, Copy, Debug)]
pub struct Folder {
    pub id: Guid,
    pub csidl: Option<u32>,
    pub base: Base,
    pub path: &'static str,
}
const fn folder(id: Guid, csidl: Option<u32>, base: Base, path: &'static str) -> Folder {
    Folder {
        id,
        csidl,
        base,
        path,
    }
}
use Base::{Fixed, LocalAppData, Profile, RoamingAppData};
pub const FOLDERS: [Folder; 41] = [
    folder(
        guid(
            0x5E6C_858F,
            0x0E22,
            0x4760,
            [0x9A, 0xFE, 0xEA, 0x33, 0x17, 0xB6, 0x71, 0x73],
        ),
        Some(0x28),
        Profile,
        "",
    ),
    folder(
        guid(
            0xB4BF_CC3A,
            0xDB2C,
            0x424C,
            [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41],
        ),
        Some(0x10),
        Profile,
        "Desktop",
    ),
    folder(
        guid(
            0xFDD3_9AD0,
            0x238F,
            0x46AF,
            [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7],
        ),
        Some(0x05),
        Profile,
        "Documents",
    ),
    folder(
        guid(
            0x374D_E290,
            0x123F,
            0x4565,
            [0x91, 0x64, 0x39, 0xC4, 0x92, 0x5E, 0x46, 0x7B],
        ),
        None,
        Profile,
        "Downloads",
    ),
    folder(
        guid(
            0x4BD8_D571,
            0x6D19,
            0x48D3,
            [0xBE, 0x97, 0x42, 0x22, 0x20, 0x08, 0x0E, 0x43],
        ),
        Some(0x0D),
        Profile,
        "Music",
    ),
    folder(
        guid(
            0x33E2_8130,
            0x4E1E,
            0x4676,
            [0x83, 0x5A, 0x98, 0x39, 0x5C, 0x3B, 0xC3, 0xBB],
        ),
        Some(0x27),
        Profile,
        "Pictures",
    ),
    folder(
        guid(
            0x1898_9B1D,
            0x99B5,
            0x455B,
            [0x84, 0x1C, 0xAB, 0x7C, 0x74, 0xE4, 0xDD, 0xFC],
        ),
        Some(0x0E),
        Profile,
        "Videos",
    ),
    folder(
        guid(
            0x1777_F761,
            0x68AD,
            0x4D8A,
            [0x87, 0xBD, 0x30, 0xB7, 0x59, 0xFA, 0x33, 0xDD],
        ),
        Some(0x06),
        Profile,
        "Favorites",
    ),
    folder(
        guid(
            0x4C5C_32FF,
            0xBB9D,
            0x43B0,
            [0xB5, 0xB4, 0x2D, 0x72, 0xE5, 0x4E, 0xAA, 0xA4],
        ),
        None,
        Profile,
        "Saved Games",
    ),
    folder(
        guid(
            0x56784854,
            0xC6CB,
            0x462B,
            [0x81, 0x69, 0x88, 0xE3, 0x50, 0xAC, 0xB8, 0x82],
        ),
        None,
        Profile,
        "Contacts",
    ),
    folder(
        guid(
            0xBFB9_D5E0,
            0xC6A9,
            0x404C,
            [0xB2, 0xB2, 0xAE, 0x6D, 0xB6, 0xAF, 0x49, 0x68],
        ),
        None,
        Profile,
        "Links",
    ),
    folder(
        guid(
            0x3EB6_85DB,
            0x65F9,
            0x4CF6,
            [0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D],
        ),
        Some(0x1A),
        RoamingAppData,
        "",
    ),
    folder(
        guid(
            0xF1B3_2785,
            0x6FBA,
            0x4FCF,
            [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
        ),
        Some(0x1C),
        LocalAppData,
        "",
    ),
    folder(
        guid(
            0xA520_A1A4,
            0x1780,
            0x4FF6,
            [0xBD, 0x18, 0x16, 0x73, 0x43, 0xC5, 0xAF, 0x16],
        ),
        None,
        Profile,
        r"AppData\LocalLow",
    ),
    folder(
        guid(
            0x5CD7_AEE2,
            0x2219,
            0x4A67,
            [0xB8, 0x5D, 0x6C, 0x9C, 0xE1, 0x56, 0x60, 0xCB],
        ),
        None,
        LocalAppData,
        "Programs",
    ),
    folder(
        guid(
            0x625B_53C3,
            0xAB48,
            0x4EC1,
            [0xBA, 0x1F, 0xA1, 0xEF, 0x41, 0x46, 0xFC, 0x19],
        ),
        Some(0x0B),
        RoamingAppData,
        r"Microsoft\Windows\Start Menu",
    ),
    folder(
        guid(
            0xA77F_5D77,
            0x2E2B,
            0x44C3,
            [0xA6, 0xA2, 0xAB, 0xA6, 0x01, 0x05, 0x4A, 0x51],
        ),
        Some(0x02),
        RoamingAppData,
        r"Microsoft\Windows\Start Menu\Programs",
    ),
    folder(
        guid(
            0xB97D_20BB,
            0xF46A,
            0x4C97,
            [0xBA, 0x10, 0x5E, 0x36, 0x08, 0x43, 0x08, 0x54],
        ),
        Some(0x07),
        RoamingAppData,
        r"Microsoft\Windows\Start Menu\Programs\Startup",
    ),
    folder(
        guid(
            0xAE50_C081,
            0xEBD2,
            0x438A,
            [0x86, 0x55, 0x8A, 0x09, 0x2E, 0x34, 0x98, 0x7A],
        ),
        Some(0x08),
        RoamingAppData,
        r"Microsoft\Windows\Recent",
    ),
    folder(
        guid(
            0x8983_036C,
            0x27C0,
            0x404B,
            [0x8F, 0x08, 0x10, 0x2D, 0x10, 0xDC, 0xFD, 0x74],
        ),
        Some(0x09),
        RoamingAppData,
        r"Microsoft\Windows\SendTo",
    ),
    folder(
        guid(
            0xA632_93E8,
            0x664E,
            0x48DB,
            [0xA0, 0x79, 0xDF, 0x75, 0x9E, 0x05, 0x09, 0xF7],
        ),
        Some(0x15),
        RoamingAppData,
        r"Microsoft\Windows\Templates",
    ),
    folder(
        guid(
            0x352481E8,
            0x33BE,
            0x4251,
            [0xBA, 0x85, 0x60, 0x07, 0xCA, 0xED, 0xCF, 0x9D],
        ),
        Some(0x20),
        LocalAppData,
        r"Microsoft\Windows\INetCache",
    ),
    folder(
        guid(
            0x2B0F_765D,
            0xC0E9,
            0x4171,
            [0x90, 0x8E, 0x08, 0xA6, 0x11, 0xB8, 0x4F, 0xF6],
        ),
        Some(0x21),
        LocalAppData,
        r"Microsoft\Windows\INetCookies",
    ),
    folder(
        guid(
            0xD9DC_8A3B,
            0xB784,
            0x432E,
            [0xA7, 0x81, 0x5A, 0x11, 0x30, 0xA7, 0x59, 0x63],
        ),
        Some(0x22),
        LocalAppData,
        r"Microsoft\Windows\History",
    ),
    folder(
        guid(
            0x62AB_5D82,
            0xFDC1,
            0x4DC3,
            [0xA9, 0xDD, 0x07, 0x0D, 0x1D, 0x49, 0x5D, 0x97],
        ),
        Some(0x23),
        Fixed,
        r"C:\ProgramData",
    ),
    folder(
        guid(
            0x0139_D44E,
            0x6AFE,
            0x49F2,
            [0x86, 0x90, 0x3D, 0xAF, 0xCA, 0xE6, 0xFF, 0xB8],
        ),
        Some(0x17),
        Fixed,
        r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs",
    ),
    folder(
        guid(
            0xA411_5719,
            0xD62E,
            0x491D,
            [0xAA, 0x7C, 0xE7, 0x4B, 0x8B, 0xE3, 0xB0, 0x67],
        ),
        Some(0x16),
        Fixed,
        r"C:\ProgramData\Microsoft\Windows\Start Menu",
    ),
    folder(
        guid(
            0x82A5_EA35,
            0xD9CD,
            0x47C5,
            [0x96, 0x29, 0xE1, 0x5D, 0x2F, 0x71, 0x4E, 0x6E],
        ),
        Some(0x18),
        Fixed,
        r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\StartUp",
    ),
    folder(
        guid(
            0x905E_63B6,
            0xC1BF,
            0x494E,
            [0xB2, 0x9C, 0x65, 0xB7, 0x32, 0xD3, 0xD2, 0x1A],
        ),
        Some(0x26),
        Fixed,
        r"C:\Program Files",
    ),
    folder(
        guid(
            0x7C5A_40EF,
            0xA0FB,
            0x4BFC,
            [0x87, 0x4A, 0xC0, 0xF2, 0xE0, 0xB9, 0xFA, 0x8E],
        ),
        Some(0x2A),
        Fixed,
        r"C:\Program Files (x86)",
    ),
    folder(
        guid(
            0xF7F1_ED05,
            0x9F6D,
            0x47A2,
            [0xAA, 0xAE, 0x29, 0xD3, 0x17, 0xC6, 0xF0, 0x66],
        ),
        Some(0x2B),
        Fixed,
        r"C:\Program Files\Common Files",
    ),
    // FOLDERID_ProgramFilesX64, ProgramFilesCommonX64 (no CSIDL; a 64-bit
    // program's own folders) and ProgramFilesCommonX86.
    folder(
        guid(
            0x6D80_9377,
            0x6AF0,
            0x444B,
            [0x89, 0x57, 0xA3, 0x77, 0x3F, 0x02, 0x20, 0x0E],
        ),
        None,
        Fixed,
        r"C:\Program Files",
    ),
    folder(
        guid(
            0x6365_D5A7,
            0x0F0D,
            0x45E5,
            [0x87, 0xF6, 0x0D, 0xA5, 0x6B, 0x6A, 0x4F, 0x7D],
        ),
        None,
        Fixed,
        r"C:\Program Files\Common Files",
    ),
    folder(
        guid(
            0xDE97_4D24,
            0xD9C6,
            0x4D3E,
            [0xBF, 0x91, 0xF4, 0x45, 0x51, 0x20, 0xB9, 0x17],
        ),
        Some(0x2C),
        Fixed,
        r"C:\Program Files (x86)\Common Files",
    ),
    folder(
        guid(
            0xF38B_F404,
            0x1D43,
            0x42F2,
            [0x93, 0x05, 0x67, 0xDE, 0x0B, 0x28, 0xFC, 0x23],
        ),
        Some(0x24),
        Fixed,
        r"C:\Windows",
    ),
    folder(
        guid(
            0x1AC1_4E77,
            0x02E7,
            0x4E5D,
            [0xB7, 0x44, 0x2E, 0xB1, 0xAE, 0x51, 0x98, 0xB7],
        ),
        Some(0x25),
        Fixed,
        r"C:\Windows\system32",
    ),
    folder(
        guid(
            0xD652_31B0,
            0xB2F1,
            0x4857,
            [0xA4, 0xCE, 0xA8, 0xE7, 0xC6, 0xEA, 0x7D, 0x27],
        ),
        Some(0x29),
        Fixed,
        r"C:\Windows\SysWOW64",
    ),
    folder(
        guid(
            0xFD22_8CB7,
            0xAE11,
            0x4AE3,
            [0x86, 0x4C, 0x16, 0xF3, 0x91, 0x0A, 0xB8, 0xFE],
        ),
        Some(0x14),
        Fixed,
        r"C:\Windows\Fonts",
    ),
    folder(
        guid(
            0xDFDF_76A2,
            0xC82A,
            0x4D63,
            [0x90, 0x6A, 0x56, 0x44, 0xAC, 0x45, 0x73, 0x85],
        ),
        None,
        Fixed,
        r"C:\Users\Public",
    ),
    folder(
        guid(
            0xC4AA_340D,
            0xF20F,
            0x4863,
            [0xAF, 0xEF, 0xF8, 0x7E, 0xF2, 0xE6, 0xBA, 0x25],
        ),
        Some(0x19),
        Fixed,
        r"C:\Users\Public\Desktop",
    ),
    folder(
        guid(
            0xED48_24AF,
            0xDCE4,
            0x45A8,
            [0x81, 0xE2, 0xFC, 0x79, 0x65, 0x08, 0x36, 0x34],
        ),
        Some(0x2E),
        Fixed,
        r"C:\Users\Public\Documents",
    ),
];
/// CSIDL_DESKTOP: the desktop (its path is the Desktop directory).
pub const CSIDL_DESKTOP: u32 = 0x00;
pub const CSIDL_FLAG_CREATE: u32 = 0x8000;
pub const CSIDL_FLAG_MASK: u32 = 0xFF00;
pub fn by_id(id: &Guid) -> Option<&'static Folder> {
    FOLDERS.iter().find(|f| f.id == *id)
}
/// The folder a 32-bit (`x86`) program is given for `f` (P17, measured
/// with wowprobe.exe): Program Files and Common Files are the (x86) ones,
/// and the 64-bit folders do not exist for it (ERROR_FILE_NOT_FOUND). A
/// 64-bit program gets `f`.
pub fn view(f: &'static Folder, x86: bool) -> Option<&'static Folder> {
    if !x86 {
        return Some(f);
    }
    // FOLDERID_ProgramFilesX64 and FOLDERID_ProgramFilesCommonX64.
    let x64 = [
        guid(
            0x6D80_9377,
            0x6AF0,
            0x444B,
            [0x89, 0x57, 0xA3, 0x77, 0x3F, 0x02, 0x20, 0x0E],
        ),
        guid(
            0x6365_D5A7,
            0x0F0D,
            0x45E5,
            [0x87, 0xF6, 0x0D, 0xA5, 0x6B, 0x6A, 0x4F, 0x7D],
        ),
    ];
    match f.csidl {
        Some(0x26) => by_csidl(0x2A),
        Some(0x2B) => by_csidl(0x2C),
        _ if x64.contains(&f.id) => None,
        _ => Some(f),
    }
}
pub fn by_csidl(csidl: u32) -> Option<&'static Folder> {
    let csidl = csidl & !CSIDL_FLAG_MASK;
    // The desktop folder's path is FOLDERID_Desktop's.
    let csidl = if csidl == CSIDL_DESKTOP { 0x10 } else { csidl };
    FOLDERS.iter().find(|f| f.csidl == Some(csidl))
}
/// The folder's path given the three environment directories.
pub fn folder_path(f: &Folder, profile: &str, roaming: &str, local: &str) -> String {
    let base = match f.base {
        Profile => profile,
        RoamingAppData => roaming,
        LocalAppData => local,
        Fixed => return String::from(f.path),
    };
    if f.path.is_empty() {
        String::from(base)
    } else {
        format!("{base}\\{}", f.path)
    }
}

/// Item ID lists. Each item is `[cb: u16][kind: u8][UTF-16 name, NUL]`, the
/// list ends with a zero `cb`; an absolute list starts with the drive item
/// ("C:\"), and the empty list is the desktop. Programs treat the bytes as
/// opaque; only shell32 reads them.
pub mod pidl {
    use alloc::{string::String, vec::Vec};
    pub const DRIVE: u8 = 0x2F;
    pub const FOLDER: u8 = 0x31;
    pub const FILE: u8 = 0x32;
    /// The list for a Windows path ("C:\a\b"); `directory` says whether the
    /// last part is a folder.
    pub fn from_path(path: &str, directory: bool) -> Option<Vec<u8>> {
        let rest = path
            .strip_prefix("C:")
            .or_else(|| path.strip_prefix("c:"))?;
        let mut out = Vec::new();
        item(&mut out, DRIVE, "C:\\");
        let parts: Vec<&str> = rest.split(['\\', '/']).filter(|p| !p.is_empty()).collect();
        for (i, part) in parts.iter().enumerate() {
            item(
                &mut out,
                if i + 1 < parts.len() || directory {
                    FOLDER
                } else {
                    FILE
                },
                part,
            );
        }
        out.extend_from_slice(&[0, 0]);
        Some(out)
    }
    fn item(out: &mut Vec<u8>, kind: u8, name: &str) {
        let units: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let cb = 2 + 1 + units.len() * 2;
        out.extend_from_slice(&(cb as u16).to_le_bytes());
        out.push(kind);
        out.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    }
    /// The items of `list` (kind, name); None when it is not a list shell32
    /// made.
    pub fn items(list: &[u8]) -> Option<Vec<(u8, String)>> {
        let mut at = 0;
        let mut out = Vec::new();
        loop {
            let cb = usize::from(u16::from_le_bytes([*list.get(at)?, *list.get(at + 1)?]));
            if cb == 0 {
                return Some(out);
            }
            let body = list.get(at + 2..at + cb)?;
            let (&kind, name) = body.split_first()?;
            if name.len() % 2 != 0 {
                return None;
            }
            let units: Vec<u16> = name
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect();
            out.push((kind, String::from_utf16(&units).ok()?));
            at += cb;
        }
    }
    /// Bytes of `list` including its terminator.
    pub fn size(list: &[u8]) -> Option<usize> {
        let mut at = 0;
        loop {
            let cb = usize::from(u16::from_le_bytes([*list.get(at)?, *list.get(at + 1)?]));
            if cb == 0 {
                return Some(at + 2);
            }
            if cb < 3 {
                return None;
            }
            at += cb;
        }
    }
    /// The Windows path of an absolute list; None for the empty (desktop)
    /// list or a foreign one.
    pub fn to_path(list: &[u8]) -> Option<String> {
        let items = items(list)?;
        let (first, rest) = items.split_first()?;
        if first.0 != DRIVE {
            return None;
        }
        let mut path = first.1.clone();
        for (i, (_, name)) in rest.iter().enumerate() {
            if i > 0 {
                path.push('\\');
            }
            path.push_str(name);
        }
        Some(path)
    }
}

/// The paths of a double-NUL-terminated list (SHFILEOPSTRUCT's pFrom/pTo).
pub fn path_list(units: &[u16]) -> Vec<String> {
    let mut out = Vec::new();
    for part in units.split(|&u| u == 0) {
        if part.is_empty() {
            break;
        }
        out.push(String::from_utf16_lossy(part));
    }
    out
}
/// A file's type name when no registered type names it, as Explorer shows
/// it: "TXT File" for an extension, "File" for none.
pub fn default_type_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
            format!("{} File", ext.to_uppercase())
        }
        _ => String::from("File"),
    }
}
/// `pattern` (`*` and `?`, case-insensitive) matches `name`.
pub fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let n: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn the_32_bit_view_of_known_folders_is_the_x86_one() {
        // Measured with wowprobe.exe under WOW64.
        let files = by_csidl(0x26).unwrap();
        assert_eq!(view(files, true).unwrap().path, r"C:\Program Files (x86)");
        assert_eq!(view(files, false).unwrap().path, r"C:\Program Files");
        let common = by_csidl(0x2B).unwrap();
        assert_eq!(
            view(common, true).unwrap().path,
            r"C:\Program Files (x86)\Common Files"
        );
        let x64 = FOLDERS
            .iter()
            .find(|f| f.csidl.is_none() && f.path == r"C:\Program Files")
            .unwrap();
        assert!(view(x64, true).is_none());
        assert_eq!(view(x64, false).unwrap().path, r"C:\Program Files");
        let system = by_csidl(0x25).unwrap();
        assert_eq!(view(system, true).unwrap().path, r"C:\Windows\system32");
        assert_eq!(by_csidl(0x29).unwrap().path, r"C:\Windows\SysWOW64");
    }
    #[test]
    fn known_folders_resolve_like_windows() {
        let documents = by_id(&FOLDERS[2].id).unwrap();
        assert_eq!(
            folder_path(documents, r"C:\Users\Ada", "r", "l"),
            r"C:\Users\Ada\Documents"
        );
        assert_eq!(
            folder_path(
                by_csidl(0x1A).unwrap(),
                "p",
                r"C:\Users\Ada\AppData\Roaming",
                "l"
            ),
            r"C:\Users\Ada\AppData\Roaming"
        );
        assert_eq!(
            folder_path(by_csidl(0x26 | CSIDL_FLAG_CREATE).unwrap(), "p", "r", "l"),
            r"C:\Program Files"
        );
        assert_eq!(
            folder_path(by_csidl(CSIDL_DESKTOP).unwrap(), r"C:\Users\Ada", "r", "l"),
            r"C:\Users\Ada\Desktop"
        );
        assert!(by_csidl(0x3F).is_none());
        // Every FOLDERID and CSIDL appears once.
        for (i, a) in FOLDERS.iter().enumerate() {
            for b in &FOLDERS[i + 1..] {
                assert_ne!(a.id, b.id);
                assert!(a.csidl.is_none() || a.csidl != b.csidl);
            }
        }
    }

    #[test]
    fn item_id_lists_round_trip() {
        let list = pidl::from_path(r"C:\Users\Ada\notes.txt", false).unwrap();
        assert_eq!(pidl::to_path(&list).unwrap(), r"C:\Users\Ada\notes.txt");
        assert_eq!(pidl::size(&list), Some(list.len()));
        let kinds: std::vec::Vec<u8> = pidl::items(&list)
            .unwrap()
            .iter()
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(kinds, [pidl::DRIVE, pidl::FOLDER, pidl::FOLDER, pidl::FILE]);
        assert_eq!(
            pidl::to_path(&pidl::from_path("C:", true).unwrap()).unwrap(),
            "C:\\"
        );
        assert_eq!(pidl::to_path(&[0, 0]), None, "the desktop has no file path");
        assert_eq!(pidl::items(&[5, 0, 1]), None, "a truncated list");
    }

    #[test]
    fn path_lists_and_type_names() {
        let units: std::vec::Vec<u16> = "a.txt\0b dir\0\0junk".encode_utf16().collect();
        assert_eq!(path_list(&units), ["a.txt", "b dir"]);
        assert_eq!(default_type_name("notes.txt"), "TXT File");
        assert_eq!(default_type_name("Makefile"), "File");
        assert_eq!(default_type_name(".hidden"), "File");
        assert!(wildcard("*.TXT", "notes.txt"));
        assert!(wildcard("a?c*", "abcdef"));
        assert!(!wildcard("*.txt", "notes.md"));
    }
}
