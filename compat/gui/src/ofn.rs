//! The Open/Save common-dialog model: `lpstrFilter` parsing, resolution of
//! the typed file name against the current folder (relative, rooted and
//! absolute paths, `.`/`..`), default extensions, the OFN_FILEMUSTEXIST/
//! OFN_PATHMUSTEXIST checks, `nFileOffset`/`nFileExtension`, and the
//! FNERR_BUFFERTOOSMALL rule. File-system queries are the caller's.
use alloc::vec::Vec;

pub mod flags {
    pub const READONLY: u32 = 0x0000_0001;
    pub const OVERWRITEPROMPT: u32 = 0x0000_0002;
    pub const HIDEREADONLY: u32 = 0x0000_0004;
    pub const NOCHANGEDIR: u32 = 0x0000_0008;
    pub const ENABLEHOOK: u32 = 0x0000_0020;
    pub const ENABLETEMPLATE: u32 = 0x0000_0040;
    pub const ENABLETEMPLATEHANDLE: u32 = 0x0000_0080;
    pub const NOVALIDATE: u32 = 0x0000_0100;
    pub const ALLOWMULTISELECT: u32 = 0x0000_0200;
    pub const PATHMUSTEXIST: u32 = 0x0000_0800;
    pub const FILEMUSTEXIST: u32 = 0x0000_1000;
    pub const CREATEPROMPT: u32 = 0x0000_2000;
    pub const EXPLORER: u32 = 0x0008_0000;
    pub const NODEREFERENCELINKS: u32 = 0x0010_0000;
    pub const DONTADDTORECENT: u32 = 0x0200_0000;
}

/// CommDlgExtendedError values used by the dialogs.
pub mod error {
    pub const STRUCTSIZE: u32 = 0x0001;
    pub const INITIALIZATION: u32 = 0x0002;
    pub const NOTEMPLATE: u32 = 0x0003;
    pub const NOHINSTANCE: u32 = 0x0004;
    pub const LOADSTRFAILURE: u32 = 0x0005;
    pub const FINDRESFAILURE: u32 = 0x0006;
    pub const LOADRESFAILURE: u32 = 0x0007;
    pub const MEMALLOCFAILURE: u32 = 0x0009;
    pub const DIALOGFAILURE: u32 = 0xffff;
    pub const SUBCLASSFAILURE: u32 = 0x3001;
    pub const INVALIDFILENAME: u32 = 0x3002;
    pub const BUFFERTOOSMALL: u32 = 0x3003;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exists {
    Missing,
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Accept {
        path: Vec<u16>,
        file_offset: u16,
        extension_offset: u16,
    },
    BufferTooSmall {
        required: usize,
        path: Vec<u16>,
    },
    /// The name is a folder: navigate into it.
    Navigate(Vec<u16>),
    /// The name contains wildcards: apply it as the current filter.
    Filter(Vec<u16>),
    NotFound(Vec<u16>),
    PathNotFound(Vec<u16>),
    InvalidName,
}

const BS: u16 = b'\\' as u16;
const FS: u16 = b'/' as u16;
const DOT: u16 = b'.' as u16;

/// Parse a double-NUL-terminated `lpstrFilter` (pairs of name/pattern).
pub fn parse_filter(units: &[u16]) -> Vec<(Vec<u16>, Vec<u16>)> {
    let mut parts = units.split(|&u| u == 0);
    let mut out = Vec::new();
    while let (Some(name), Some(pattern)) = (parts.next(), parts.next()) {
        if name.is_empty() || pattern.is_empty() || out.len() >= 64 {
            break;
        }
        out.push((name.to_vec(), pattern.to_vec()));
    }
    out
}

/// Does `name` match one `;`-separated wildcard pattern list?
pub fn matches(patterns: &[u16], name: &[u16]) -> bool {
    patterns
        .split(|&u| u == u16::from(b';'))
        .any(|p| glob(p, name))
}
fn glob(p: &[u16], n: &[u16]) -> bool {
    let fold = crate::geom::wide::fold;
    match (p.first(), n.first()) {
        (None, None) => true,
        // "*.*" also matches names without a dot, as in Windows.
        (Some(&42), _) if p == [42, DOT, 42] => true,
        (Some(&42), _) => glob(&p[1..], n) || (!n.is_empty() && glob(p, &n[1..])),
        (Some(&63), Some(_)) => glob(&p[1..], &n[1..]),
        (Some(&a), Some(&b)) if fold(a) == fold(b) => glob(&p[1..], &n[1..]),
        _ => false,
    }
}

fn is_absolute(t: &[u16]) -> bool {
    t.len() >= 3 && t[1] == u16::from(b':') && (t[2] == BS || t[2] == FS)
}

/// Join and normalize; `None` for an invalid name.
pub fn join(current: &[u16], typed: &[u16]) -> Option<Vec<u16>> {
    let typed: Vec<u16> = typed
        .iter()
        .map(|&u| if u == FS { BS } else { u })
        .collect();
    let invalid = [b'<', b'>', b'"', b'|'].map(u16::from);
    if typed.iter().any(|u| invalid.contains(u) || *u < 32) {
        return None;
    }
    let base: Vec<u16> = if is_absolute(&typed) {
        typed[..2].to_vec()
    } else if typed.first() == Some(&BS) {
        current
            .get(..2)
            .filter(|d| d.get(1) == Some(&u16::from(b':')))?
            .to_vec()
    } else {
        current.to_vec()
    };
    let rest = if is_absolute(&typed) {
        &typed[2..]
    } else {
        &typed[..]
    };
    let mut segments: Vec<Vec<u16>> = Vec::new();
    let drive_len = if base.len() >= 2 && base[1] == u16::from(b':') {
        2
    } else {
        0
    };
    for s in base[drive_len..]
        .split(|&u| u == BS)
        .chain(rest.split(|&u| u == BS))
    {
        match s {
            [] => (),
            [DOT] => (),
            [DOT, DOT] => {
                segments.pop();
            }
            _ => {
                if s.contains(&u16::from(b':')) {
                    return None;
                }
                segments.push(s.to_vec());
            }
        }
    }
    let mut out = base[..drive_len].to_vec();
    if segments.is_empty() {
        out.push(BS);
    }
    for s in segments {
        out.push(BS);
        out.extend(s);
    }
    Some(out)
}

pub struct Request<'a> {
    pub flags: u32,
    pub save: bool,
    pub default_extension: Option<&'a [u16]>,
    /// Capacity of `lpstrFile` in characters (`nMaxFile`).
    pub capacity: usize,
}

/// Resolve the typed name for OK.
pub fn resolve(
    req: &Request,
    current: &[u16],
    typed: &[u16],
    exists: &dyn Fn(&[u16]) -> Exists,
) -> Outcome {
    let trimmed: &[u16] = {
        let s = typed.iter().position(|&u| u != 32).unwrap_or(typed.len());
        let e = typed.iter().rposition(|&u| u != 32).map_or(s, |e| e + 1);
        &typed[s..e]
    };
    if trimmed.is_empty() {
        return Outcome::InvalidName;
    }
    if trimmed.iter().any(|&u| u == 42 || u == 63) {
        return Outcome::Filter(trimmed.to_vec());
    }
    let Some(mut path) = join(current, trimmed) else {
        return Outcome::InvalidName;
    };
    if exists(&path) == Exists::Directory {
        return Outcome::Navigate(path);
    }
    let file_offset = path.iter().rposition(|&u| u == BS).map_or(0, |p| p + 1);
    if path.len() == file_offset {
        return Outcome::InvalidName;
    }
    let has_extension = path[file_offset..].contains(&DOT);
    if !has_extension && let Some(ext) = req.default_extension.filter(|e| !e.is_empty()) {
        let candidate: Vec<u16> = path
            .iter()
            .copied()
            .chain(core::iter::once(DOT))
            .chain(ext.iter().copied())
            .collect();
        // An open dialog keeps the bare name if only that exists.
        if req.save || exists(&candidate) != Exists::Missing || exists(&path) == Exists::Missing {
            path = candidate;
        }
    }
    let parent = &path[..file_offset.saturating_sub(1).max(2)];
    if req.flags & (flags::PATHMUSTEXIST | flags::FILEMUSTEXIST) != 0
        && file_offset > 3
        && exists(parent) != Exists::Directory
    {
        return Outcome::PathNotFound(path);
    }
    if req.flags & flags::FILEMUSTEXIST != 0 && exists(&path) != Exists::File {
        return Outcome::NotFound(path);
    }
    let required = path.len() + 1;
    if required > req.capacity {
        return Outcome::BufferTooSmall { required, path };
    }
    let extension_offset = path[file_offset..]
        .iter()
        .rposition(|&u| u == DOT)
        .map_or(0, |p| file_offset + p + 1);
    Outcome::Accept {
        file_offset: file_offset as u16,
        extension_offset: extension_offset as u16,
        path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    #[test]
    fn filters_parse_and_match() {
        let f = parse_filter(&w(
            "Text documents (*.txt)\0*.txt\0All files (*.*)\0*.*\0\0",
        ));
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].1, w("*.txt"));
        assert!(matches(&f[0].1, &w("A.TXT")));
        assert!(!matches(&f[0].1, &w("a.txt.bak")));
        assert!(matches(&f[1].1, &w("README")));
        assert!(matches(&w("*.c;*.h"), &w("x.h")));
        assert!(matches(&w("?b*"), &w("abc")));
        assert!(parse_filter(&w("only-name\0")).is_empty());
    }
    #[test]
    fn paths_join_and_normalize() {
        let cur = w("C:\\users\\1001");
        assert_eq!(join(&cur, &w("a.txt")), Some(w("C:\\users\\1001\\a.txt")));
        assert_eq!(join(&cur, &w("..\\b.txt")), Some(w("C:\\users\\b.txt")));
        assert_eq!(join(&cur, &w("\\x\\.\\y")), Some(w("C:\\x\\y")));
        assert_eq!(join(&cur, &w("D:/z")), Some(w("D:\\z")));
        assert_eq!(join(&cur, &w("..\\..\\..\\..")), Some(w("C:\\")));
        assert_eq!(join(&cur, &w("a|b")), None);
        assert_eq!(join(&cur, &w("a:b")), None);
    }
    #[test]
    fn save_and_open_resolution() {
        let cur = w("C:\\users\\1001");
        let name = w("wingui-selftest-\u{3bb}.txt");
        let fs = |p: &[u16]| {
            if p == w("C:\\users\\1001") || p == w("C:\\users\\1001\\docs") {
                Exists::Directory
            } else if p == w("C:\\users\\1001\\wingui-selftest-\u{3bb}.txt") {
                Exists::File
            } else {
                Exists::Missing
            }
        };
        let save = Request {
            flags: flags::EXPLORER,
            save: true,
            default_extension: Some(&[116, 120, 116]),
            capacity: 324,
        };
        match resolve(&save, &cur, &name, &fs) {
            Outcome::Accept {
                path,
                file_offset,
                extension_offset,
            } => {
                assert_eq!(path.len(), cur.len() + 1 + name.len());
                assert_eq!(file_offset as usize, cur.len() + 1);
                assert_eq!(extension_offset as usize, cur.len() + 1 + name.len() - 3);
            }
            other => panic!("{other:?}"),
        }
        match resolve(&save, &cur, &w("notes"), &fs) {
            Outcome::Accept { path, .. } => assert_eq!(path, w("C:\\users\\1001\\notes.txt")),
            other => panic!("{other:?}"),
        }
        let tiny = Request {
            capacity: 8,
            ..save
        };
        match resolve(&tiny, &cur, &w("wingui-selftest-long-name.txt"), &fs) {
            Outcome::BufferTooSmall { required, .. } => assert!(required > 8),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            resolve(&save, &cur, &w("docs"), &fs),
            Outcome::Navigate(w("C:\\users\\1001\\docs"))
        );
        assert_eq!(
            resolve(&save, &cur, &w("*.c"), &fs),
            Outcome::Filter(w("*.c"))
        );
        assert_eq!(resolve(&save, &cur, &w("   "), &fs), Outcome::InvalidName);
        let open = Request {
            flags: flags::FILEMUSTEXIST | flags::PATHMUSTEXIST,
            save: false,
            default_extension: Some(&[116, 120, 116]),
            capacity: 324,
        };
        assert!(matches!(
            resolve(&open, &cur, &name, &fs),
            Outcome::Accept { .. }
        ));
        assert!(matches!(
            resolve(&open, &cur, &w("missing.txt"), &fs),
            Outcome::NotFound(_)
        ));
        assert!(matches!(
            resolve(&open, &cur, &w("nodir\\x.txt"), &fs),
            Outcome::PathNotFound(_)
        ));
    }
}
