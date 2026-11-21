#![no_std]
//! The client transport WinHTTP and WinINet share: name resolution, TCP
//! connections with timeouts over ntdll's `DoorNet*` sockets, TLS 1.3 with
//! the Windows certificate-error flags (door-tls), and one HTTP/1.1
//! exchange at a time over it (door-http). Blocking; the DLLs run it on
//! their own worker threads for asynchronous handles.
extern crate alloc;
use alloc::string::String;

/// Why a connection or exchange failed, before each DLL maps it to its
/// ERROR_WINHTTP_* / ERROR_INTERNET_* code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    NameNotResolved,
    CannotConnect,
    Timeout,
    /// The connection broke (reset, closed mid-response).
    ConnectionError,
    /// The server's TLS handshake or certificate failed: the
    /// `WINHTTP_CALLBACK_STATUS_FLAG_*` bits say how.
    Secure(u32),
    InvalidResponse,
    Cancelled,
    OutOfMemory,
}

/// WINHTTP_CALLBACK_STATUS_FLAG_* / INTERNET security failure bits.
pub mod secure {
    pub const CERT_REV_FAILED: u32 = 0x1;
    pub const INVALID_CERT: u32 = 0x2;
    pub const CERT_REVOKED: u32 = 0x4;
    pub const INVALID_CA: u32 = 0x8;
    pub const CERT_CN_INVALID: u32 = 0x10;
    pub const CERT_DATE_INVALID: u32 = 0x20;
    pub const CERT_WRONG_USAGE: u32 = 0x40;
    pub const SECURITY_CHANNEL_ERROR: u32 = 0x8000_0000;
}

/// Which certificate problems the caller accepts (SECURITY_FLAG_IGNORE_*).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tolerance {
    pub unknown_ca: bool,
    pub date: bool,
    pub common_name: bool,
    pub wrong_usage: bool,
}
impl Tolerance {
    /// From SECURITY_FLAG_IGNORE_* bits (same values in WinHTTP and WinINet).
    pub fn from_flags(flags: u32) -> Self {
        Self {
            unknown_ca: flags & 0x100 != 0,
            date: flags & 0x2000 != 0,
            common_name: flags & 0x1000 != 0,
            wrong_usage: flags & 0x200 != 0,
        }
    }
    pub fn policy(self) -> door_tls::verify::Policy {
        door_tls::verify::Policy {
            unknown_ca: self.unknown_ca,
            date: self.date,
            host: self.common_name,
            usage: self.wrong_usage,
        }
    }
}
/// The WINHTTP_CALLBACK_STATUS_FLAG_* bits of the certificate problems
/// door-tls found.
pub fn failure_flags(problems: door_tls::verify::Problems) -> u32 {
    let mut flags = 0;
    if problems.untrusted {
        flags |= secure::INVALID_CA;
    }
    if problems.date {
        flags |= secure::CERT_DATE_INVALID;
    }
    if problems.host {
        flags |= secure::CERT_CN_INVALID;
    }
    if problems.usage {
        flags |= secure::CERT_WRONG_USAGE;
    }
    if problems.malformed || problems.signature {
        flags |= secure::INVALID_CERT;
    }
    flags
}
/// `failure_flags` of the problems `policy` does not accept: what a
/// refused handshake's SECURE_FAILURE reports (Windows leaves out the
/// errors the request's SECURITY_FLAG_IGNORE_* accepted).
pub fn refused_flags(
    problems: door_tls::verify::Problems,
    policy: door_tls::verify::Policy,
) -> u32 {
    failure_flags(door_tls::verify::Problems {
        untrusted: problems.untrusted && !policy.unknown_ca,
        date: problems.date && !policy.date,
        host: problems.host && !policy.host,
        usage: problems.usage && !policy.usage,
        ..problems
    })
}

/// Strict dotted-quad IPv4 text.
pub fn ipv4_literal(text: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = text.split('.');
    for slot in &mut out {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// A proxy `host:port` (a scheme prefix is dropped; port 80 when absent).
pub fn parse_proxy(text: &str) -> Option<(String, u16)> {
    let text = text.split([';', ' ']).find(|t| !t.is_empty())?;
    let text = text.split_once("://").map_or(text, |(_, rest)| rest);
    // "http=host:port" entries name a proxy for one scheme.
    let text = text.split_once('=').map_or(text, |(_, rest)| rest);
    let (host, port) = match text.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (text, 80),
    };
    (!host.is_empty()).then(|| (String::from(host), port))
}
/// Whether `host` matches a proxy bypass list (`;`- or space-separated
/// entries, `*` wildcards, `<local>` for names without a dot).
pub fn bypassed(host: &str, list: &str) -> bool {
    list.split([';', ' ', ','])
        .filter(|e| !e.is_empty())
        .any(|entry| {
            if entry.eq_ignore_ascii_case("<local>") {
                return !host.contains('.');
            }
            wildcard(&entry.to_ascii_lowercase(), &host.to_ascii_lowercase())
        })
}
fn wildcard(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((head, tail)) => {
            text.starts_with(head)
                && (0..=text.len() - head.len()).any(|i| wildcard(tail, &text[head.len() + i..]))
        }
    }
}

/// Pointer-width conversions for the Windows exports this crate calls
/// (P17), as door_windows_abi::widen and narrow do them: a 32-bit value is
/// sign-extended, so -1/-2 handles keep their meaning.
#[allow(dead_code)]
pub(crate) const fn widen(value: usize) -> u64 {
    value as isize as i64 as u64
}
#[allow(dead_code)]
pub(crate) const fn narrow(value: u64) -> usize {
    value as usize
}
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
pub mod autoproxy;
pub mod cache;
pub mod cookies;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
pub mod exchange;
pub mod ftp;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
pub mod guest;
pub mod proxyconfig;
pub mod sync;

#[cfg(test)]
mod tests {
    use super::*;
    /// Windows 11's WINHTTP_CALLBACK_STATUS_SECURE_FAILURE reasons against
    /// CONNECT-A's self-signed certificates (httpprobe steps 17-19): an
    /// untrusted root alone; with IGNORE_UNKNOWN_CA, only the wrong host or
    /// only the expired date.
    #[test]
    fn refusal_reasons_leave_out_ignored_problems() {
        use door_tls::verify::{Policy, Problems};
        let untrusted = Problems {
            untrusted: true,
            ..Problems::default()
        };
        assert_eq!(
            refused_flags(untrusted, Policy::default()),
            secure::INVALID_CA
        );
        let unknown = Policy {
            unknown_ca: true,
            ..Policy::default()
        };
        let wrong_host = Problems {
            host: true,
            ..untrusted
        };
        assert_eq!(refused_flags(wrong_host, unknown), secure::CERT_CN_INVALID);
        let expired = Problems {
            date: true,
            ..untrusted
        };
        assert_eq!(refused_flags(expired, unknown), secure::CERT_DATE_INVALID);
        assert_eq!(
            failure_flags(wrong_host),
            secure::INVALID_CA | secure::CERT_CN_INVALID
        );
    }
    #[test]
    fn proxies_and_bypass_lists() {
        assert_eq!(
            parse_proxy("proxy.corp:8080"),
            Some((String::from("proxy.corp"), 8080))
        );
        assert_eq!(
            parse_proxy("http=p:3128;https=q:3129"),
            Some((String::from("p"), 3128))
        );
        assert_eq!(parse_proxy("http://p"), Some((String::from("p"), 80)));
        assert_eq!(parse_proxy(""), None);
        assert!(bypassed("intranet", "<local>"));
        assert!(!bypassed("www.example.com", "<local>"));
        assert!(bypassed("a.corp.example", "*.corp.example;10.*"));
        assert!(bypassed("10.0.2.2", "*.corp.example;10.*"));
        assert!(!bypassed("example.org", "*.corp.example;10.*"));
        assert_eq!(
            Tolerance::from_flags(0x3300),
            Tolerance {
                unknown_ca: true,
                date: true,
                common_name: true,
                wrong_usage: true
            }
        );
    }
}
