#![no_std]
//! Original wintrust.dll: WinVerifyTrust for embedded Authenticode
//! signatures and catalog members (WINTRUST_ACTION_GENERIC_VERIFY_V2 and
//! DRIVER_ACTION_VERIFY), with revocation by CRL when asked
//! (WTD_REVOKE_WHOLECHAIN or the WTD_REVOCATION_CHECK_* provider flags),
//! and the CryptCATAdmin functions over the catalog store
//! C:\Windows\System32\CatRoot\{subsystem GUID}. Verification is
//! `door_authenticode`'s; trust anchors are its code-signing roots, the
//! Mozilla roots and the certificates of the machine's Trusted Root store
//! (HKLM\SOFTWARE\Microsoft\SystemCertificates\ROOT\Certificates), as
//! Windows' chain engine trusts them. CRLs are fetched over WinHTTP and
//! kept in the user's CryptnetUrlCache, as cryptnet does.
//!
//! This module is the host-testable part: the GUIDs, flags and codes
//! Windows uses and the store's folder names. The exports are in `guest`.
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

/// HRESULTs WinVerifyTrust returns (winerror.h).
pub mod hr {
    pub const S_OK: u32 = 0;
    pub const TRUST_E_PROVIDER_UNKNOWN: u32 = 0x800B_0001;
    pub const TRUST_E_ACTION_UNKNOWN: u32 = 0x800B_0002;
    pub const TRUST_E_SUBJECT_FORM_UNKNOWN: u32 = 0x800B_0003;
    pub const TRUST_E_NOSIGNATURE: u32 = 0x800B_0100;
    pub const E_INVALIDARG: u32 = 0x8007_0057;
    /// HRESULT_FROM_WIN32.
    pub const fn win32(code: u32) -> u32 {
        if code == 0 {
            0
        } else {
            0x8007_0000 | (code & 0xFFFF)
        }
    }
}
/// GUIDs (their 16 bytes in memory order).
pub mod action {
    /// WINTRUST_ACTION_GENERIC_VERIFY_V2 {00AAC56B-CD44-11D0-8CC2-00C04FC295EE}.
    pub const GENERIC_VERIFY_V2: [u8; 16] = guid(
        0x00AA_C56B,
        0xCD44,
        0x11D0,
        [0x8C, 0xC2, 0x00, 0xC0, 0x4F, 0xC2, 0x95, 0xEE],
    );
    /// DRIVER_ACTION_VERIFY {F750E6C3-38EE-11D1-85E5-00C04FC295EE}, also the
    /// subsystem of the system's catalogs.
    pub const DRIVER_VERIFY: [u8; 16] = guid(
        0xF750_E6C3,
        0x38EE,
        0x11D1,
        [0x85, 0xE5, 0x00, 0xC0, 0x4F, 0xC2, 0x95, 0xEE],
    );
    pub const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
        let a = a.to_le_bytes();
        let b = b.to_le_bytes();
        let c = c.to_le_bytes();
        [
            a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5],
            d[6], d[7],
        ]
    }
}
/// WINTRUST_DATA's values this implementation reads.
pub mod data {
    pub const REVOKE_NONE: u32 = 0;
    pub const REVOKE_WHOLECHAIN: u32 = 1;
    pub const CHOICE_FILE: u32 = 1;
    pub const CHOICE_CATALOG: u32 = 2;
    pub const STATE_IGNORE: u32 = 0;
    pub const STATE_VERIFY: u32 = 1;
    pub const STATE_CLOSE: u32 = 2;
    pub const PROV_REVOCATION_CHECK_NONE: u32 = 0x10;
    pub const PROV_REVOCATION_CHECK_END_CERT: u32 = 0x20;
    pub const PROV_REVOCATION_CHECK_CHAIN: u32 = 0x40;
    pub const PROV_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT: u32 = 0x80;
    pub const PROV_CACHE_ONLY_URL_RETRIEVAL: u32 = 0x1000;
    /// Whether a call asks for revocation checking.
    pub const fn revocation(checks: u32, flags: u32) -> bool {
        (checks == REVOKE_WHOLECHAIN
            || flags
                & (PROV_REVOCATION_CHECK_END_CERT
                    | PROV_REVOCATION_CHECK_CHAIN
                    | PROV_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT)
                != 0)
            && flags & PROV_REVOCATION_CHECK_NONE == 0
    }
}
/// The catalog store folder of a subsystem GUID: CatRoot\{GUID}.
pub fn store_folder(subsystem: &[u8; 16]) -> String {
    let field = |r: core::ops::Range<usize>| -> String {
        subsystem[r]
            .iter()
            .rev()
            .map(|x| alloc::format!("{x:02X}"))
            .collect()
    };
    let tail: String = subsystem[8..]
        .iter()
        .map(|x| alloc::format!("{x:02X}"))
        .collect();
    alloc::format!(
        "C:\\Windows\\System32\\CatRoot\\{{{}-{}-{}-{}-{}}}",
        field(0..4),
        field(4..6),
        field(6..8),
        &tail[..4],
        &tail[4..]
    )
}
/// The file name cryptnet caches a URL's content under (the SHA-1 of the
/// URL, hexadecimal).
pub fn cache_name(url: &str) -> String {
    use sha1::Digest;
    sha1::Sha1::digest(url.as_bytes())
        .iter()
        .map(|b| alloc::format!("{b:02X}"))
        .collect()
}
/// `http://host[:port]/path` as (host, port, path).
pub fn split_url(url: &str) -> Option<(&str, u16, &str)> {
    let rest = url
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("http://"))
        .and(url.get(7..))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok()?),
        None => (authority, 80),
    };
    (!host.is_empty()).then_some((host, port, path))
}
/// A catalog file name CryptCATAdminAddCatalog accepts: a plain name.
pub fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['\\', '/', ':']) && name != "." && name != ".."
}
/// The hashes a catalog names files by for one hash algorithm.
pub fn pick(hashes: (Vec<u8>, Vec<u8>), sha256: bool) -> Vec<u8> {
    if sha256 { hashes.1 } else { hashes.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folders_urls_and_flags() {
        assert_eq!(
            store_folder(&action::DRIVER_VERIFY),
            "C:\\Windows\\System32\\CatRoot\\{F750E6C3-38EE-11D1-85E5-00C04FC295EE}"
        );
        assert_eq!(
            split_url("http://10.0.2.2:28480/doors-trust-test.crl"),
            Some(("10.0.2.2", 28480, "/doors-trust-test.crl"))
        );
        assert_eq!(
            split_url("HTTP://crl.example"),
            Some(("crl.example", 80, "/"))
        );
        assert_eq!(split_url("https://crl.example/a.crl"), None);
        assert_eq!(split_url("http://:80/"), None);
        assert_eq!(
            cache_name("abc"),
            "A9993E364706816ABA3E25717850C26C9CD0D89D"
        );
        assert!(data::revocation(data::REVOKE_WHOLECHAIN, 0));
        assert!(data::revocation(0, data::PROV_REVOCATION_CHECK_CHAIN));
        assert!(!data::revocation(
            data::REVOKE_WHOLECHAIN,
            data::PROV_REVOCATION_CHECK_NONE
        ));
        assert!(!data::revocation(
            data::REVOKE_NONE,
            data::PROV_CACHE_ONLY_URL_RETRIEVAL
        ));
        assert!(plain_name("doors-trust-test.cat"));
        assert!(!plain_name("..\\x.cat") && !plain_name("C:x.cat") && !plain_name(".."));
        assert_eq!(hr::win32(2), 0x8007_0002);
    }
}
