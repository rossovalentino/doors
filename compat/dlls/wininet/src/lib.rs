#![no_std]
//! Original WinINet (wininet.dll): InternetOpen/Connect, HttpOpenRequest,
//! HttpSendRequest(Ex), InternetReadFile, InternetOpenUrl, headers, options,
//! status callbacks, cookies and URL helpers, in ANSI and Unicode, over the
//! transport and HTTP engine shared with WinHTTP (door-windows-inet).
//!
//! This module is the host-testable part: InternetCrackUrl's scheme table
//! and component rules (measured on Windows 11 26H2), canonicalization and
//! the error codes.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;
use alloc::{string::String, vec::Vec};

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// ERROR_INTERNET_* codes.
pub mod error {
    pub const INVALID_HANDLE: u32 = 6;
    pub const NOT_ENOUGH_MEMORY: u32 = 8;
    pub const INVALID_PARAMETER: u32 = 87;
    pub const INSUFFICIENT_BUFFER: u32 = 122;
    pub const NO_MORE_ITEMS: u32 = 259;
    pub const NOT_LOCKED: u32 = 158;
    pub const IO_PENDING: u32 = 997;
    pub const FILE_NOT_FOUND: u32 = 2;
    pub const TIMEOUT: u32 = 12002;
    pub const INTERNAL_ERROR: u32 = 12004;
    pub const INVALID_URL: u32 = 12005;
    pub const UNRECOGNIZED_SCHEME: u32 = 12006;
    pub const NAME_NOT_RESOLVED: u32 = 12007;
    pub const PROTOCOL_NOT_FOUND: u32 = 12008;
    pub const INVALID_OPTION: u32 = 12009;
    pub const BAD_OPTION_LENGTH: u32 = 12010;
    pub const OPTION_NOT_SETTABLE: u32 = 12011;
    pub const INVALID_OPERATION: u32 = 12016;
    pub const OPERATION_CANCELLED: u32 = 12017;
    pub const INCORRECT_HANDLE_TYPE: u32 = 12018;
    pub const INCORRECT_HANDLE_STATE: u32 = 12019;
    pub const CANNOT_CONNECT: u32 = 12029;
    pub const CONNECTION_ABORTED: u32 = 12030;
    pub const CONNECTION_RESET: u32 = 12031;
    pub const SEC_CERT_DATE_INVALID: u32 = 12037;
    pub const SEC_CERT_CN_INVALID: u32 = 12038;
    pub const HTTPS_TO_HTTP_ON_REDIR: u32 = 12040;
    pub const INVALID_CA: u32 = 12045;
    pub const SEC_CERT_ERRORS: u32 = 12055;
    pub const HTTP_HEADER_NOT_FOUND: u32 = 12150;
    pub const HTTP_INVALID_SERVER_RESPONSE: u32 = 12152;
    pub const HTTP_INVALID_HEADER: u32 = 12153;
    pub const HTTP_INVALID_QUERY_REQUEST: u32 = 12154;
    pub const HTTP_HEADER_ALREADY_EXISTS: u32 = 12155;
    pub const HTTP_REDIRECT_FAILED: u32 = 12156;
    pub const SECURITY_CHANNEL_ERROR: u32 = 12157;
    pub const SEC_INVALID_CERT: u32 = 12169;
}

/// INTERNET_SCHEME values (WinINet's numbering, not WinHTTP's).
pub mod scheme {
    pub const UNKNOWN: i32 = -1;
    pub const FTP: i32 = 1;
    pub const GOPHER: i32 = 2;
    pub const HTTP: i32 = 3;
    pub const HTTPS: i32 = 4;
    pub const FILE: i32 = 5;
    pub const NEWS: i32 = 6;
    pub const MAILTO: i32 = 7;
    pub const SOCKS: i32 = 8;
    pub const JAVASCRIPT: i32 = 9;
    pub const VBSCRIPT: i32 = 10;
    pub const RES: i32 = 11;
}
/// The scheme number and default port of a scheme name.
pub fn scheme_of(name: &str) -> Option<(i32, u16)> {
    Some(match name.to_ascii_lowercase().as_str() {
        "ftp" => (scheme::FTP, 21),
        "gopher" => (scheme::GOPHER, 0),
        "http" => (scheme::HTTP, 80),
        "https" => (scheme::HTTPS, 443),
        "file" => (scheme::FILE, 0),
        "news" => (scheme::NEWS, 0),
        "mailto" => (scheme::MAILTO, 0),
        "socks" => (scheme::SOCKS, 0),
        "javascript" => (scheme::JAVASCRIPT, 0),
        "vbscript" => (scheme::VBSCRIPT, 0),
        "res" => (scheme::RES, 0),
        _ => return None,
    })
}

pub type Span = (usize, usize);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cracked {
    pub scheme: Span,
    pub number: i32,
    pub host: Span,
    pub port: u16,
    pub user: Span,
    pub password: Span,
    pub path: Span,
    pub extra: Span,
}
fn position(units: &[u16], from: usize, set: &[u8]) -> usize {
    units[from..]
        .iter()
        .position(|u| set.iter().any(|s| u16::from(*s) == *u))
        .map_or(units.len(), |i| from + i)
}
/// `InternetCrackUrl`'s split (measured on Windows 11 26H2): http-like
/// schemes as WinHTTP's (with WinINet's scheme numbers and ftp's port 21);
/// `file://` has an empty host and the rest as path; `mailto:user@host`
/// splits at `@`; an unknown scheme is ERROR_INTERNET_UNRECOGNIZED_SCHEME.
pub fn crack(units: &[u16]) -> Result<Cracked, u32> {
    let colon = units
        .iter()
        .position(|u| *u == u16::from(b':'))
        .ok_or(error::UNRECOGNIZED_SCHEME)?;
    let name = String::from_utf16_lossy(&units[..colon]);
    let (number, default_port) = scheme_of(&name).ok_or(error::UNRECOGNIZED_SCHEME)?;
    let mut c = Cracked {
        scheme: (0, colon),
        number,
        port: default_port,
        ..Cracked::default()
    };
    if number == scheme::MAILTO {
        let start = colon + 1;
        let at = position(units, start, b"@");
        if at < units.len() {
            c.user = (start, at - start);
            c.host = (at + 1, units.len() - at - 1);
        } else {
            c.path = (start, units.len() - start);
        }
        return Ok(c);
    }
    let slashes = units.get(colon + 1..colon + 3) == Some(&[u16::from(b'/'), u16::from(b'/')][..]);
    if !slashes {
        c.path = (colon + 1, units.len() - colon - 1);
        return Ok(c);
    }
    let start = colon + 3;
    if number == scheme::FILE {
        c.host = (start, 0);
        c.path = (start, units.len() - start);
        return Ok(c);
    }
    let authority_end = position(units, start, b"/?#");
    let at = units[start..authority_end]
        .iter()
        .rposition(|u| *u == u16::from(b'@'))
        .map(|i| start + i);
    let host_start = match at {
        Some(at) => {
            let colon = position(units, start, b":").min(at);
            if colon < at {
                c.user = (start, colon - start);
                c.password = (colon + 1, at - colon - 1);
            } else {
                c.user = (start, at - start);
            }
            at + 1
        }
        None => start,
    };
    let port_colon = position(units, host_start, b":").min(authority_end);
    if port_colon == host_start {
        return Err(error::INVALID_URL);
    }
    c.host = (host_start, port_colon - host_start);
    if port_colon < authority_end {
        let digits = String::from_utf16_lossy(&units[port_colon + 1..authority_end]);
        if !digits.is_empty() {
            c.port = digits.parse::<u16>().map_err(|_| error::INVALID_URL)?;
        }
    }
    let extra = position(units, authority_end, b"?#");
    c.path = (authority_end, extra - authority_end);
    c.extra = (extra, units.len() - extra);
    Ok(c)
}

/// `InternetCanonicalizeUrl` (default flags, measured): dot segments of
/// the path are removed and unsafe path characters percent-encoded; the
/// query is left as is.
pub fn canonicalize(url: &str) -> Result<String, u32> {
    let parsed = door_http::parse_url(url).map_err(|e| match e {
        door_http::UrlError::UnrecognizedScheme => error::UNRECOGNIZED_SCHEME,
        door_http::UrlError::Invalid => error::INVALID_URL,
    })?;
    // Resolving "" against the URL itself removes dot segments.
    let mut base = door_http::resolve(&parsed, &parsed.path).map_err(|_| error::INVALID_URL)?;
    base.fragment = parsed.fragment.clone();
    let (path, query) = match base.path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (base.path.as_str(), None),
    };
    let mut encoded = String::new();
    for b in path.bytes() {
        if b <= b' ' || b >= 0x7f || b"\"<>\\^`{|}".contains(&b) {
            encoded.push_str(&alloc::format!("%{b:02X}"));
        } else {
            encoded.push(char::from(b));
        }
    }
    let mut text = alloc::format!("{}://", base.scheme.name());
    if !base.user.is_empty() {
        text.push_str(&base.user);
        if !base.password.is_empty() {
            text.push(':');
            text.push_str(&base.password);
        }
        text.push('@');
    }
    text.push_str(&base.host_header());
    text.push_str(&encoded);
    if let Some(q) = query {
        text.push('?');
        text.push_str(q);
    }
    if !base.fragment.is_empty() {
        text.push('#');
        text.push_str(&base.fragment);
    }
    Ok(text)
}
/// `InternetCombineUrl`: RFC 3986 reference resolution.
pub fn combine(base: &str, relative: &str) -> Result<String, u32> {
    let base = door_http::parse_url(base).map_err(|_| error::INVALID_URL)?;
    door_http::resolve(&base, relative)
        .map(|u| u.to_text())
        .map_err(|_| error::INVALID_URL)
}
/// ANSI text (the system code page) as Unicode: Latin-1 plus Windows-1252's
/// 0x80..0x9F.
pub fn from_ansi(bytes: &[u8]) -> String {
    const HIGH: [u16; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013,
        0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    let units: Vec<u16> = bytes
        .iter()
        .map(|b| {
            if (0x80..0xA0).contains(b) {
                HIGH[usize::from(b - 0x80)]
            } else {
                u16::from(*b)
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}
pub fn to_ansi(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match u32::from(c) {
            v @ (0..=0x7f | 0xa0..=0xff) => v as u8,
            0x20AC => 0x80,
            0x2018 => 0x91,
            0x2019 => 0x92,
            0x201C => 0x93,
            0x201D => 0x94,
            0x2013 => 0x96,
            0x2014 => 0x97,
            _ => b'?',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn part(u: &[u16], (s, n): Span) -> String {
        String::from_utf16_lossy(&u[s..s + n])
    }
    /// Measured with InternetCrackUrlW on this PC.
    #[test]
    fn crack_matches_windows() {
        let u = units("https://user:pw@Example.COM:8443/a/b?c=d#frag");
        let c = crack(&u).unwrap();
        assert_eq!((c.number, c.port), (scheme::HTTPS, 8443));
        assert_eq!(
            [
                part(&u, c.host),
                part(&u, c.user),
                part(&u, c.password),
                part(&u, c.path),
                part(&u, c.extra)
            ],
            ["Example.COM", "user", "pw", "/a/b", "?c=d#frag"]
        );
        let u = units("http://h?x=1");
        let c = crack(&u).unwrap();
        assert_eq!(
            (c.number, c.port, part(&u, c.path), part(&u, c.extra)),
            (scheme::HTTP, 80, String::new(), String::from("?x=1"))
        );
        let u = units("ftp://example.com/");
        assert_eq!(crack(&u).map(|c| (c.number, c.port)), Ok((scheme::FTP, 21)));
        assert_eq!(
            crack(&units("example.com")),
            Err(error::UNRECOGNIZED_SCHEME)
        );
        let u = units("file:///C:/x/y.txt");
        let c = crack(&u).unwrap();
        assert_eq!(
            (c.number, c.host.1, part(&u, c.path)),
            (scheme::FILE, 0, String::from("/C:/x/y.txt"))
        );
        let u = units("mailto:a@b.c");
        let c = crack(&u).unwrap();
        assert_eq!(
            (c.number, part(&u, c.user), part(&u, c.host)),
            (scheme::MAILTO, String::from("a"), String::from("b.c"))
        );
        assert_eq!(crack(&units("http://h:0/")).map(|c| c.port), Ok(0));
        assert_eq!(crack(&units("http://:80/")), Err(error::INVALID_URL));
        assert_eq!(
            crack(&units("gopher://h/")).map(|c| (c.number, c.port)),
            Ok((scheme::GOPHER, 0))
        );
    }
    #[test]
    fn helpers_match_windows() {
        assert_eq!(
            canonicalize("http://h/a/./b/../c d?x y").unwrap(),
            "http://h/a/c%20d?x y"
        );
        assert_eq!(
            combine("http://a/b/c/d;p?q", "../g").unwrap(),
            "http://a/b/g"
        );
        assert_eq!(from_ansi(b"caf\xe9 \x80"), "caf\u{e9} \u{20ac}");
        assert_eq!(to_ansi("caf\u{e9} \u{20ac}"), b"caf\xe9 \x80");
    }
}
