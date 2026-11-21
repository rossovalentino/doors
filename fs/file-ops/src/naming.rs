//! Windows File Explorer naming for kept-both and same-folder copies.
use crate::{Error, NAME_BYTES, Name, Result};

/// UTF-16 units of `text` (the namespace's component length unit).
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}
/// Split "name.ext" into ("name", ".ext"); folders and dot-files keep no
/// extension.
fn split(name: &str, directory: bool) -> (&str, &str) {
    if directory {
        return (name, "");
    }
    match name.rfind('.') {
        Some(at) if at > 0 => name.split_at(at),
        _ => (name, ""),
    }
}
/// `base + middle + ext`, truncating `base` (on a char boundary) so the
/// result fits 48 UTF-16 units.
fn compose(base: &str, middle: &str, ext: &str) -> Result<Name> {
    let budget = 48usize
        .checked_sub(utf16_len(middle) + utf16_len(ext))
        .ok_or(Error::Invalid)?;
    let mut end = 0;
    let mut units = 0;
    for (at, c) in base.char_indices() {
        if units + c.len_utf16() > budget {
            break;
        }
        units += c.len_utf16();
        end = at + c.len_utf8();
    }
    if end == 0 {
        return Err(Error::Invalid);
    }
    let mut out = [0u8; NAME_BYTES * 2];
    let parts = [&base[..end], middle, ext];
    let mut len = 0;
    for part in parts {
        out[len..len + part.len()].copy_from_slice(part.as_bytes());
        len += part.len();
    }
    Name::new(core::str::from_utf8(&out[..len]).map_err(|_| Error::Invalid)?)
}
/// "name (n).ext" for n >= 2 ("name.ext" for n < 2), as File Explorer's
/// "Keep both" and restore collisions name the incoming item.
pub fn numbered_name(name: &str, directory: bool, n: u32) -> Result<Name> {
    let (base, ext) = split(name, directory);
    if n < 2 {
        return compose(base, "", ext);
    }
    let mut digits = [0u8; 16];
    let middle = format_middle(&mut digits, " (", n, ")");
    compose(base, middle, ext)
}
/// "name - Copy.ext", then "name - Copy (n).ext": a copy pasted into its own
/// folder.
pub fn copy_name(name: &str, directory: bool, n: u32) -> Result<Name> {
    let (base, ext) = split(name, directory);
    if n < 2 {
        return compose(base, " - Copy", ext);
    }
    let mut digits = [0u8; 24];
    let middle = format_middle(&mut digits, " - Copy (", n, ")");
    compose(base, middle, ext)
}
fn format_middle<'a>(out: &'a mut [u8], prefix: &str, n: u32, suffix: &str) -> &'a str {
    let mut len = prefix.len();
    out[..len].copy_from_slice(prefix.as_bytes());
    let mut tmp = [0u8; 10];
    let mut v = n;
    let mut count = 0;
    loop {
        tmp[count] = b'0' + (v % 10) as u8;
        count += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for i in (0..count).rev() {
        out[len] = tmp[i];
        len += 1;
    }
    out[len..len + suffix.len()].copy_from_slice(suffix.as_bytes());
    len += suffix.len();
    core::str::from_utf8(&out[..len]).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_keep_both_and_copy_names() {
        assert_eq!(
            numbered_name("report.txt", false, 2).unwrap().as_str(),
            "report (2).txt"
        );
        assert_eq!(
            numbered_name("Folder.v1", true, 3).unwrap().as_str(),
            "Folder.v1 (3)"
        );
        assert_eq!(
            numbered_name(".profile", false, 2).unwrap().as_str(),
            ".profile (2)"
        );
        assert_eq!(
            copy_name("a.b.txt", false, 1).unwrap().as_str(),
            "a.b - Copy.txt"
        );
        assert_eq!(
            copy_name("Docs", true, 12).unwrap().as_str(),
            "Docs - Copy (12)"
        );
        // Long names are truncated to fit 48 UTF-16 units, keeping the suffix.
        let long = "x".repeat(46) + ".md";
        let n = numbered_name(&long, false, 2).unwrap();
        assert_eq!(utf16_len(n.as_str()), 48);
        assert!(n.as_str().ends_with(" (2).md"));
        let wide = "\u{1F600}".repeat(24);
        let n = copy_name(&wide, true, 1).unwrap();
        assert!(utf16_len(n.as_str()) <= 48 && n.as_str().ends_with(" - Copy"));
    }
}
