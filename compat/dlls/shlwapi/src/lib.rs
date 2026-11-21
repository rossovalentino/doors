#![no_std]
//! Original shlwapi.dll (P18): the path helpers (on UTF-16 units, as
//! Windows' work in place), PathMatchSpec's wildcards, association queries
//! over HKCR (AssocQueryString), QISearch (ordinal 219) and
//! SHCreateMemStream (ordinal 12, an IStream over a copy of the bytes).
//!
//! The same sources build the x64 and the x86 DLL.
extern crate alloc;
use alloc::vec::Vec;

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// The exports importers use by ordinal (shlwapi.dll's export table).
pub const ORDINALS: [(&str, u16); 2] = [("SHCreateMemStream", 12), ("QISearch", 219)];

const DOT: u16 = b'.' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const COLON: u16 = b':' as u16;
const SPACE: u16 = b' ' as u16;
const SEMICOLON: u16 = b';' as u16;

fn separator(u: u16) -> bool {
    u == BACKSLASH || u == SLASH || u == COLON
}
/// PathFindFileName: the index of the last component (after the last
/// `\`, `/` or `:`; a trailing separator is part of the component).
pub fn file_name(path: &[u16]) -> usize {
    let mut start = 0;
    for (i, &u) in path.iter().enumerate() {
        if separator(u) && i + 1 < path.len() && !separator(path[i + 1]) {
            start = i + 1;
        }
    }
    start
}
/// PathFindExtension: the index of the last `.` of the file name (no
/// spaces after it), else the path's end.
pub fn extension(path: &[u16]) -> usize {
    let name = file_name(path);
    let mut found = path.len();
    for (i, &u) in path.iter().enumerate().skip(name) {
        if u == DOT {
            found = i;
        } else if u == SPACE || separator(u) {
            found = path.len();
        }
    }
    found
}
fn fold(u: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&u) {
        u - 32
    } else {
        u
    }
}
/// One `*`/`?` pattern against a name, case-insensitively.
fn wildcard(pattern: &[u16], name: &[u16]) -> bool {
    let (mut p, mut n, mut star, mut mark) = (0, 0, None, 0);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == u16::from(b'?') || fold(pattern[p]) == fold(name[n]))
        {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == u16::from(b'*') {
            star = Some(p);
            p += 1;
            mark = n;
        } else if let Some(s) = star {
            p = s + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&u| u == u16::from(b'*'))
}
/// PathMatchSpec: the file name of `path` against any `;`-separated
/// pattern of `spec` (leading spaces ignored); `*.*` matches every name.
pub fn matches(path: &[u16], spec: &[u16]) -> bool {
    let name = &path[file_name(path)..];
    spec.split(|&u| u == SEMICOLON).any(|p| {
        let p: Vec<u16> = p.iter().copied().skip_while(|&u| u == SPACE).collect();
        let star_dot_star = [u16::from(b'*'), DOT, u16::from(b'*')];
        !p.is_empty() && (p == star_dot_star || wildcard(&p, name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    #[test]
    fn finds_names_and_extensions() {
        let p = w(r"C:\Users\ada\notes.v2.txt");
        assert_eq!(&p[file_name(&p)..], &w("notes.v2.txt")[..]);
        assert_eq!(&p[extension(&p)..], &w(".txt")[..]);
        let q = w(r"C:\folder.d\name");
        assert_eq!(extension(&q), q.len());
        assert_eq!(file_name(&w(r"C:\")), 0);
    }
    #[test]
    fn matches_specs() {
        assert!(matches(&w(r"C:\a\Report.DOCX"), &w("*.txt; *.docx")));
        assert!(!matches(&w(r"C:\a\Report.doc"), &w("*.docx")));
        assert!(matches(&w("x"), &w("*.*")));
        assert!(matches(&w("photo1.png"), &w("photo?.png")));
    }
}
