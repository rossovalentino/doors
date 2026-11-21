#![no_std]
//! Original crypt32.dll: certificate stores and contexts, and data
//! protection. Memory, system and collection stores hold the certificates
//! a program adds (CertAddEncodedCertificateToStore, CryptQueryObject);
//! the system stores start empty, as on a computer with no certificates
//! installed beyond the browser's own roots, and are not persisted.
//! Contexts carry Windows' CERT_CONTEXT/CERT_INFO layout (`cert`). Names
//! render for CertGetNameString; signatures verify through door-tls.
//! Certificate chains are not built (CertGetCertificateChain reports none):
//! Chromium and .NET verify with their own engines. DPAPI (`dpapi`) seals
//! data under per-user or per-computer keys kept in the registry.
//!
//! The same sources build the x64 and the x86 DLL.
extern crate alloc;

pub mod cert;
pub mod dpapi;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

pub const CRYPT_E_NOT_FOUND: u32 = 0x8009_2004;
pub const CRYPT_E_EXISTS: u32 = 0x8009_2005;
pub const CRYPT_E_NO_MATCH: u32 = 0x8009_2009;
pub const CRYPT_E_NO_KEY_PROPERTY: u32 = 0x8009_200B;
pub const CRYPT_E_INVALID_MSG_TYPE: u32 = 0x8009_100C;
pub const NTE_BAD_SIGNATURE: u32 = 0x8009_0006;
pub const NTE_BAD_ALGID: u32 = 0x8009_0008;
pub const E_ACCESSDENIED: u32 = 0x8007_0005;
pub const ERROR_INVALID_DATA: u32 = 13;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_MORE_DATA: u32 = 234;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const CRYPT_E_ASN1_BADTAG: u32 = 0x8009_310B;
pub const E_INVALIDARG: u32 = 0x8007_0057;

/// The ALG_ID of an algorithm object identifier (CertOIDToAlgId), 0 for
/// one this table does not know.
pub fn alg_id(oid: &[u8]) -> u32 {
    match oid {
        b"1.2.840.113549.2.2" => 0x8001,      // MD2
        b"1.2.840.113549.2.4" => 0x8002,      // MD4
        b"1.2.840.113549.2.5" => 0x8003,      // MD5
        b"1.3.14.3.2.26" => 0x8004,           // SHA-1
        b"2.16.840.1.101.3.4.2.1" => 0x800c,  // SHA-256
        b"2.16.840.1.101.3.4.2.2" => 0x800d,  // SHA-384
        b"2.16.840.1.101.3.4.2.3" => 0x800e,  // SHA-512
        b"1.2.840.113549.1.1.1" => 0xa400,    // RSA key exchange
        b"1.2.840.113549.1.1.5" => 0x8004,    // sha1RSA
        b"1.2.840.113549.1.1.11" => 0x800c,   // sha256RSA
        b"1.2.840.113549.1.1.12" => 0x800d,   // sha384RSA
        b"1.2.840.113549.1.1.13" => 0x800e,   // sha512RSA
        b"1.2.840.10040.4.1" => 0x2200,       // DSA
        b"1.2.840.10045.2.1" => 0x0000_aa03,  // ECDH/ECC public key
        b"1.2.840.10045.4.3.2" => 0x800c,     // ecdsa-with-SHA256
        b"1.2.840.10045.4.3.3" => 0x800d,     // ecdsa-with-SHA384
        b"1.2.840.10045.4.3.4" => 0x800e,     // ecdsa-with-SHA512
        b"2.16.840.1.101.3.4.1.2" => 0x660e,  // AES-128-CBC
        b"2.16.840.1.101.3.4.1.42" => 0x6610, // AES-256-CBC
        b"1.2.840.113549.3.7" => 0x6603,      // 3DES
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_and_key_oids_map_to_their_algorithms() {
        assert_eq!(alg_id(b"1.3.14.3.2.26"), 0x8004);
        assert_eq!(alg_id(b"2.16.840.1.101.3.4.2.1"), 0x800c);
        assert_eq!(alg_id(b"1.2.840.113549.1.1.1"), 0xa400);
        assert_eq!(alg_id(b"1.2.3"), 0);
    }
}
