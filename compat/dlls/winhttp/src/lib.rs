#![no_std]
//! Original WinHTTP (winhttp.dll) over door.network and door-tls: sessions,
//! connections and requests in synchronous and asynchronous mode, status
//! callbacks, request and response headers, redirects, cookies, Basic
//! authentication, proxies, timeouts and TLS 1.3 with WinHTTP's security
//! flags. The HTTP/1.1 engine is `door-http`.
//!
//! This module is the host-testable part: `WinHttpCrackUrl`/`CreateUrl`
//! component rules (measured on Windows 11 26H2), the cookie jar and the
//! error codes.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;
use alloc::string::String;

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// ERROR_WINHTTP_* and the Win32 errors WinHTTP reports.
pub mod error {
    pub const NOT_ENOUGH_MEMORY: u32 = 8;
    pub const INVALID_PARAMETER: u32 = 87;
    pub const INSUFFICIENT_BUFFER: u32 = 122;
    pub const INVALID_HANDLE: u32 = 6;
    pub const OUT_OF_HANDLES: u32 = 12001;
    pub const TIMEOUT: u32 = 12002;
    pub const INTERNAL_ERROR: u32 = 12004;
    pub const INVALID_URL: u32 = 12005;
    pub const UNRECOGNIZED_SCHEME: u32 = 12006;
    pub const NAME_NOT_RESOLVED: u32 = 12007;
    pub const INVALID_OPTION: u32 = 12009;
    pub const OPTION_NOT_SETTABLE: u32 = 12011;
    pub const SHUTDOWN: u32 = 12012;
    pub const LOGIN_FAILURE: u32 = 12015;
    pub const OPERATION_CANCELLED: u32 = 12017;
    pub const INCORRECT_HANDLE_TYPE: u32 = 12018;
    pub const INCORRECT_HANDLE_STATE: u32 = 12019;
    pub const CANNOT_CONNECT: u32 = 12029;
    pub const CONNECTION_ERROR: u32 = 12030;
    pub const RESEND_REQUEST: u32 = 12032;
    pub const SECURE_CERT_DATE_INVALID: u32 = 12037;
    pub const SECURE_CERT_CN_INVALID: u32 = 12038;
    pub const CLIENT_AUTH_CERT_NEEDED: u32 = 12044;
    pub const SECURE_INVALID_CA: u32 = 12045;
    pub const SECURE_CERT_REV_FAILED: u32 = 12057;
    pub const CANNOT_CALL_BEFORE_OPEN: u32 = 12100;
    pub const CANNOT_CALL_BEFORE_SEND: u32 = 12101;
    pub const CANNOT_CALL_AFTER_SEND: u32 = 12102;
    pub const CANNOT_CALL_AFTER_OPEN: u32 = 12103;
    pub const HEADER_NOT_FOUND: u32 = 12150;
    pub const INVALID_SERVER_RESPONSE: u32 = 12152;
    pub const INVALID_HEADER: u32 = 12153;
    pub const INVALID_QUERY_REQUEST: u32 = 12154;
    pub const HEADER_ALREADY_EXISTS: u32 = 12155;
    pub const REDIRECT_FAILED: u32 = 12156;
    pub const SECURE_CHANNEL_ERROR: u32 = 12157;
    pub const BAD_AUTO_PROXY_SCRIPT: u32 = 12166;
    pub const UNABLE_TO_DOWNLOAD_SCRIPT: u32 = 12167;
    pub const SECURE_INVALID_CERT: u32 = 12169;
    pub const SECURE_CERT_REVOKED: u32 = 12170;
    pub const NOT_INITIALIZED: u32 = 12172;
    pub const SECURE_FAILURE: u32 = 12175;
    pub const AUTO_PROXY_SERVICE_ERROR: u32 = 12178;
    pub const SECURE_CERT_WRONG_USAGE: u32 = 12179;
    pub const AUTODETECTION_FAILED: u32 = 12180;
    pub const HEADER_COUNT_EXCEEDED: u32 = 12181;
    pub const HEADER_SIZE_OVERFLOW: u32 = 12182;
    pub const CHUNKED_ENCODING_HEADER_SIZE_OVERFLOW: u32 = 12183;
    pub const RESPONSE_DRAIN_OVERFLOW: u32 = 12184;
}

/// INTERNET_SCHEME values.
pub mod scheme {
    pub const HTTP: i32 = 1;
    pub const HTTPS: i32 = 2;
    pub const FTP: i32 = 3;
    pub const SOCKS: i32 = 4;
}

/// One cracked component: (start, length) in UTF-16 units of the input.
pub type Span = (usize, usize);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cracked {
    pub scheme: Span,
    pub scheme_number: i32,
    pub host: Span,
    pub port: u16,
    pub user: Span,
    pub password: Span,
    pub path: Span,
    pub extra: Span,
}
fn index_of(units: &[u16], from: usize, set: &[u16]) -> usize {
    units[from..]
        .iter()
        .position(|u| set.contains(u))
        .map_or(units.len(), |i| from + i)
}
/// `WinHttpCrackUrl`'s split, measured on Windows 11 26H2: the scheme
/// before `://` (http, https, ftp, socks are known; anything else or none
/// is ERROR_WINHTTP_UNRECOGNIZED_SCHEME), user and password before `@`, the
/// host (empty: ERROR_WINHTTP_INVALID_URL), the port (0 allowed; over
/// 65535 or not a number: INVALID_URL; absent: the scheme's default), the
/// path up to `?`/`#` (empty when absent) and the extra info from there.
pub fn crack(units: &[u16]) -> Result<Cracked, u32> {
    let colon = units
        .iter()
        .position(|u| *u == u16::from(b':'))
        .ok_or(error::UNRECOGNIZED_SCHEME)?;
    let scheme_text: String = String::from_utf16_lossy(&units[..colon]).to_ascii_lowercase();
    let (number, default_port) = match scheme_text.as_str() {
        "http" => (scheme::HTTP, 80),
        "https" => (scheme::HTTPS, 443),
        "ftp" => (scheme::FTP, 0),
        "socks" => (scheme::SOCKS, 0),
        _ => return Err(error::UNRECOGNIZED_SCHEME),
    };
    let slashes = [u16::from(b'/'), u16::from(b'/')];
    if units.get(colon + 1..colon + 3) != Some(&slashes[..]) {
        return Err(error::INVALID_URL);
    }
    let start = colon + 3;
    let authority_end = index_of(
        units,
        start,
        &[u16::from(b'/'), u16::from(b'?'), u16::from(b'#')],
    );
    let authority = &units[start..authority_end];
    let mut c = Cracked {
        scheme: (0, colon),
        scheme_number: number,
        port: default_port,
        ..Cracked::default()
    };
    let host_start = match authority.iter().rposition(|u| *u == u16::from(b'@')) {
        Some(at) => {
            let userinfo = &authority[..at];
            match userinfo.iter().position(|u| *u == u16::from(b':')) {
                Some(p) => {
                    c.user = (start, p);
                    c.password = (start + p + 1, at - p - 1);
                }
                None => c.user = (start, at),
            }
            start + at + 1
        }
        None => start,
    };
    let hostport = &units[host_start..authority_end];
    let port_colon = if hostport.first() == Some(&u16::from(b'[')) {
        let close = hostport
            .iter()
            .position(|u| *u == u16::from(b']'))
            .ok_or(error::INVALID_URL)?;
        hostport[close..]
            .iter()
            .position(|u| *u == u16::from(b':'))
            .map(|i| close + i)
    } else {
        hostport.iter().position(|u| *u == u16::from(b':'))
    };
    let host_length = port_colon.unwrap_or(hostport.len());
    if host_length == 0 {
        return Err(error::INVALID_URL);
    }
    c.host = (host_start, host_length);
    if let Some(p) = port_colon {
        let digits = &hostport[p + 1..];
        if !digits.is_empty() {
            let mut value: u32 = 0;
            for d in digits {
                if !(u16::from(b'0')..=u16::from(b'9')).contains(d) {
                    return Err(error::INVALID_URL);
                }
                value = value * 10 + u32::from(d - u16::from(b'0'));
                if value > 65535 {
                    return Err(error::INVALID_URL);
                }
            }
            c.port = value as u16;
        }
    }
    let extra_start = index_of(units, authority_end, &[u16::from(b'?'), u16::from(b'#')]);
    c.path = (authority_end, extra_start - authority_end);
    c.extra = (extra_start, units.len() - extra_start);
    Ok(c)
}

/// The components `WinHttpCreateUrl` joins.
pub struct Parts<'a> {
    pub scheme: &'a str,
    pub number: i32,
    pub host: &'a str,
    pub port: u16,
    pub user: &'a str,
    pub password: &'a str,
    pub path: &'a str,
    pub extra: &'a str,
}
/// `WinHttpCreateUrl`: scheme://[user[:password]@]host[:port]path extra;
/// the port only when it differs from the scheme's default.
pub fn create(p: &Parts<'_>) -> String {
    let Parts {
        scheme: scheme_name,
        number,
        host,
        port,
        user,
        password,
        path,
        extra,
    } = *p;
    let default = match number {
        scheme::HTTP => 80,
        scheme::HTTPS => 443,
        _ => 0,
    };
    let mut s = alloc::format!("{scheme_name}://");
    if !user.is_empty() {
        s.push_str(user);
        if !password.is_empty() {
            s.push(':');
            s.push_str(password);
        }
        s.push('@');
    }
    s.push_str(host);
    if port != default && port != 0 {
        s.push_str(&alloc::format!(":{port}"));
    }
    s.push_str(path);
    s.push_str(extra);
    s
}

pub use door_windows_inet::cookies::{Cookie, Jar};

#[cfg(test)]
mod tests;
