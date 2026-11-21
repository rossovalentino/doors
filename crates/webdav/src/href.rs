//! Relative paths <-> hrefs. NoneDrive's paths are `/`-separated, without
//! a leading or trailing slash, in Unicode; on the wire every byte outside
//! RFC 3986's unreserved set is percent-encoded, segment by segment.
use crate::Error;
use alloc::{string::String, vec::Vec};

fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}
/// Percent-encode `rel` (segments kept, `/` between them).
pub fn encode(rel: &str) -> Result<String, Error> {
    if !valid_relative(rel) {
        return Err(Error::Path);
    }
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(rel.len());
    for b in rel.bytes() {
        if unreserved(b) || b == b'/' {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 15)]));
        }
    }
    Ok(out)
}
/// A relative path: segments neither empty, `.` nor `..`, no `\` and no
/// control characters.
pub fn valid_relative(rel: &str) -> bool {
    rel.is_empty()
        || rel.split('/').all(|s| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && !s.contains('\\')
                && !s.chars().any(char::is_control)
        })
}
/// Percent-decode `text` into UTF-8 (None for a bad escape or invalid
/// UTF-8).
pub fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
/// An href from a listing (an absolute path or a full URL) as a path
/// relative to the sync folder `base` (itself percent-encoded, ending with
/// `/`): Some("") for the folder itself, None when outside it. A trailing
/// `/` is dropped; the caller learns folders from `resourcetype`.
pub fn relative(href: &str, base: &str) -> Option<String> {
    let mut path = href.trim();
    if let Some(rest) = path
        .strip_prefix("https://")
        .or_else(|| path.strip_prefix("http://"))
    {
        path = &rest[rest.find('/')?..];
    }
    let path = decode(path)?;
    let base = decode(base)?;
    let rest = if path.len() + 1 == base.len() && base.starts_with(path.as_str()) {
        // The folder itself without its trailing slash.
        ""
    } else {
        strip_prefix_folded(&path, &base)?
    };
    let rest = rest.trim_end_matches('/');
    valid_relative(rest).then(|| String::from(rest))
}
/// `path` without `prefix`; servers may change the case of the base path
/// (Windows' own WebDAV server does), so the prefix compares without case.
fn strip_prefix_folded<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let head = path.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &path[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_round_trip_through_percent_encoding() {
        let rel = "Fotos 2026/Città & mare/ü+#?.jpg";
        let e = encode(rel).unwrap();
        assert_eq!(
            e,
            "Fotos%202026/Citt%C3%A0%20%26%20mare/%C3%BC%2B%23%3F.jpg"
        );
        assert_eq!(decode(&e).as_deref(), Some(rel));
        assert_eq!(encode("a/../b"), Err(Error::Path));
        assert_eq!(encode("/a"), Err(Error::Path));
        assert_eq!(encode("a//b"), Err(Error::Path));
        assert_eq!(decode("%zz"), None);
        assert_eq!(decode("%C3"), None);
    }
    #[test]
    fn hrefs_become_paths_relative_to_the_sync_folder() {
        let base = "/remote.php/dav/files/me/NoneDrive/";
        assert_eq!(relative(base, base).as_deref(), Some(""));
        assert_eq!(
            relative("/remote.php/dav/files/me/NoneDrive", base).as_deref(),
            Some("")
        );
        assert_eq!(
            relative("/remote.php/dav/files/me/NoneDrive/Docs/", base).as_deref(),
            Some("Docs")
        );
        assert_eq!(
            relative(
                "https://nas.home:8443/remote.php/dav/files/me/NoneDrive/a%20b.txt",
                base
            )
            .as_deref(),
            Some("a b.txt")
        );
        assert_eq!(
            relative("/REMOTE.php/dav/files/me/NoneDrive/x", base).as_deref(),
            Some("x")
        );
        assert_eq!(relative("/elsewhere/x", base), None);
        assert_eq!(relative("/remote.php/dav/files/me/NoneDrive/../x", base), None);
    }
}
