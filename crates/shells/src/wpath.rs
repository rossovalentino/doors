//! Windows path arithmetic as `GetFullPathNameW` performs it for drive
//! paths: `/` is `\`, `C:x` is relative to C:'s current directory, `\x` to
//! the current drive's root, `.`/`..` fold, and trailing dots and spaces
//! leave each component. UNC and `\\?\` paths are not part of Doors.
use crate::host::FsError;
use alloc::{format, string::String, vec::Vec};

/// Device names that are never files.
pub fn device(name: &str) -> Option<&'static str> {
    let stem = name.split('.').next().unwrap_or("").trim_end();
    for d in ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"] {
        if stem.eq_ignore_ascii_case(d) {
            return Some(d);
        }
    }
    for p in ["COM", "LPT"] {
        if stem.len() == 4
            && stem[..3].eq_ignore_ascii_case(p)
            && matches!(stem.as_bytes()[3], b'1'..=b'9')
        {
            return Some(if p == "COM" { "COM" } else { "LPT" });
        }
    }
    None
}
/// The drive letter of a full path, upper-case.
pub fn drive(path: &str) -> Option<char> {
    let b = path.as_bytes();
    (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic())
        .then(|| (b[0] as char).to_ascii_uppercase())
}
/// Resolve `input` against `cwd` (a full path) and the other drives'
/// current directories (`drive_cwd`).
pub fn full(
    cwd: &str,
    drive_cwd: &dyn Fn(char) -> Option<String>,
    input: &str,
) -> Result<String, FsError> {
    let input = input.replace('/', "\\");
    if input.starts_with("\\\\") {
        return Err(FsError::Unsupported);
    }
    if input
        .chars()
        .any(|c| c < ' ' || matches!(c, '<' | '>' | '|' | '"'))
    {
        return Err(FsError::Invalid);
    }
    let current = drive(cwd).unwrap_or('C');
    let (base, rest) = if let Some(d) = drive(&input) {
        let rest = &input[2..];
        if rest.starts_with('\\') {
            (format!("{d}:\\"), rest.to_string_owned())
        } else if d == current {
            (String::from(cwd), rest.to_string_owned())
        } else {
            (
                drive_cwd(d).unwrap_or_else(|| format!("{d}:\\")),
                rest.to_string_owned(),
            )
        }
    } else if input.starts_with('\\') {
        (format!("{current}:\\"), input.clone())
    } else {
        (String::from(cwd), input.clone())
    };
    let d = drive(&base).ok_or(FsError::Invalid)?;
    let mut parts: Vec<String> = Vec::new();
    for p in base[2..].split('\\').chain(rest.split('\\')) {
        match p {
            "" | "." => (),
            ".." => {
                parts.pop();
            }
            _ => {
                let t = p.trim_end_matches(['.', ' ']);
                if t.is_empty() {
                    // "..." folds like "." on Windows.
                    continue;
                }
                if t.contains(':') {
                    return Err(FsError::Invalid);
                }
                parts.push(String::from(t));
            }
        }
    }
    let mut out = format!("{d}:\\");
    out.push_str(&parts.join("\\"));
    Ok(out)
}
trait Owned {
    fn to_string_owned(&self) -> String;
}
impl Owned for str {
    fn to_string_owned(&self) -> String {
        String::from(self)
    }
}
/// `C:\a\b.txt` -> (`C:\a`, `b.txt`); the root's parent is itself.
pub fn split(path: &str) -> (&str, &str) {
    match path.rfind('\\') {
        Some(i) if i <= 2 => (&path[..3.min(path.len())], &path[i + 1..]),
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    }
}
pub fn parent(path: &str) -> &str {
    split(path).0
}
pub fn name(path: &str) -> &str {
    split(path).1
}
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') {
        format!("{dir}{name}")
    } else {
        format!("{dir}\\{name}")
    }
}
/// Extension with its dot (`.txt`), empty when none.
pub fn extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 || name.len() > 1 => &name[i..],
        _ => "",
    }
}
pub fn stem(name: &str) -> &str {
    let e = extension(name);
    &name[..name.len() - e.len()]
}
pub fn is_root(path: &str) -> bool {
    path.len() == 3 && path.ends_with(":\\")
}
/// Has `*` or `?`.
pub fn wild(path: &str) -> bool {
    path.contains(['*', '?'])
}
/// The 8.3 short name Windows would generate for a long name (first
/// collision slot), as `dir /x` and `%~s` show it.
pub fn short_name(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    let (stem, ext) = match upper.rfind('.') {
        Some(i) if i > 0 => (&upper[..i], &upper[i + 1..]),
        _ => (upper.as_str(), ""),
    };
    let clean = |s: &str| {
        s.chars()
            .filter(|c| !matches!(c, ' ' | '.' | '+' | ',' | ';' | '=' | '[' | ']'))
            .collect::<String>()
    };
    let s = clean(stem);
    let e: String = clean(ext).chars().take(3).collect();
    let fits = s.len() <= 8
        && ext.len() <= 3
        && name.chars().all(|c| c.is_ascii() && c != ' ')
        && name.matches('.').count() <= 1;
    if fits {
        return upper;
    }
    let base: String = s.chars().take(6).collect();
    if e.is_empty() {
        format!("{base}~1")
    } else {
        format!("{base}~1.{e}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn f(cwd: &str, p: &str) -> String {
        full(cwd, &|d| (d == 'D').then(|| String::from("D:\\work")), p).unwrap()
    }
    #[test]
    fn full_paths_follow_getfullpathname() {
        assert_eq!(f("C:\\Users\\a", "x.txt"), "C:\\Users\\a\\x.txt");
        assert_eq!(f("C:\\Users\\a", "..\\..\\Windows"), "C:\\Windows");
        assert_eq!(
            f("C:\\Users\\a", "\\Windows/System32"),
            "C:\\Windows\\System32"
        );
        assert_eq!(f("C:\\Users\\a", "C:"), "C:\\Users\\a");
        assert_eq!(f("C:\\Users\\a", "D:x"), "D:\\work\\x");
        assert_eq!(f("C:\\Users\\a", "c:\\"), "C:\\");
        assert_eq!(f("C:\\", ".."), "C:\\");
        assert_eq!(f("C:\\Users", "a. .\\b"), "C:\\Users\\a\\b");
        assert!(full("C:\\", &|_| None, "\\\\srv\\share").is_err());
        assert!(full("C:\\", &|_| None, "a|b").is_err());
    }
    #[test]
    fn helpers() {
        assert_eq!(split("C:\\a\\b.txt"), ("C:\\a", "b.txt"));
        assert_eq!(split("C:\\b.txt"), ("C:\\", "b.txt"));
        assert_eq!(extension("a.tar.gz"), ".gz");
        assert_eq!(extension(".gitignore"), ".gitignore");
        assert_eq!(stem("report.docx"), "report");
        assert_eq!(device("nul.txt"), Some("NUL"));
        assert_eq!(device("com1"), Some("COM"));
        assert_eq!(device("console"), None);
        assert_eq!(short_name("Program Files"), "PROGRA~1");
        assert_eq!(short_name("a.txt"), "A.TXT");
        assert_eq!(short_name("long name.html"), "LONGNA~1.HTM");
    }
}
