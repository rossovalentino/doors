//! Code-signing trust anchors. Windows trusts code through its own root
//! program, which includes roots the Mozilla TLS set does not: Microsoft's
//! own roots and the public CAs' dedicated code-signing and legacy
//! time-stamping roots. Those ship here next to the Mozilla roots (which
//! still anchor many code-signing chains: DigiCert, USERTrust, Certum,
//! Entrust). Certificates are public data; provenance is in
//! `roots/README.md`.
use alloc::vec::Vec;
use door_tls::verify::Anchor;

/// Microsoft's code-signing and time-stamping roots.
const MICROSOFT: [&[u8]; 6] = [
    include_bytes!("../roots/microsoft-root-2010.der"),
    include_bytes!("../roots/microsoft-root-2011.der"),
    include_bytes!("../roots/microsoft-time-stamp-root-2014.der"),
    include_bytes!("../roots/microsoft-ecc-ts-root-2018.der"),
    include_bytes!("../roots/microsoft-ecc-product-root-2018.der"),
    include_bytes!("../roots/microsoft-identity-verification-root-2020.der"),
];
/// Public CAs' dedicated code-signing roots and the legacy roots older
/// time-stamping authorities chain to.
const PUBLIC: [&[u8]; 7] = [
    include_bytes!("../roots/sectigo-code-signing-root-r46.der"),
    include_bytes!("../roots/sectigo-code-signing-root-e46.der"),
    include_bytes!("../roots/digicert-cs-rsa4096-root-g5.der"),
    include_bytes!("../roots/globalsign-code-signing-root-r45.der"),
    include_bytes!("../roots/verisign-universal-root.der"),
    include_bytes!("../roots/verisign-class3-g5.der"),
    include_bytes!("../roots/thawte-primary-root.der"),
];
/// The code-signing roots that are not in the Mozilla set.
pub fn anchors() -> Vec<Anchor<'static>> {
    MICROSOFT
        .iter()
        .chain(PUBLIC.iter())
        .filter_map(|der| door_tls::x509::parse(der))
        .map(|c| Anchor {
            subject: c.subject,
            spki: c.spki,
            name_constraints: None,
        })
        .collect()
}
/// Every anchor a signature may chain to.
pub fn trusted() -> Vec<Anchor<'static>> {
    let mut all = door_tls::verify::mozilla_roots();
    all.extend(anchors());
    all
}
/// The certificate of a machine Trusted Root store entry: the serialized
/// properties Windows keeps in
/// `HKLM\SOFTWARE\Microsoft\SystemCertificates\ROOT\Certificates\<SHA-1>\Blob`
/// (`[id u32][1 u32][length u32][data]` each), whose CERT_CERT_PROP_ID
/// (32) is the DER certificate.
pub fn store_certificate(blob: &[u8]) -> Option<&[u8]> {
    let mut at = 0;
    while at + 12 <= blob.len() {
        let id = u32::from_le_bytes(blob[at..at + 4].try_into().ok()?);
        let length = u32::from_le_bytes(blob[at + 8..at + 12].try_into().ok()?) as usize;
        let data = blob.get(at + 12..at.checked_add(12 + length)?)?;
        if id == 32 {
            return Some(data);
        }
        at += 12 + length;
    }
    None
}
/// Anchors for certificates (DER) a store holds; ones that do not parse
/// are left out.
pub fn of(certificates: &[Vec<u8>]) -> Vec<Anchor<'_>> {
    certificates
        .iter()
        .filter_map(|der| door_tls::x509::parse(der))
        .map(|c| Anchor {
            subject: c.subject,
            spki: c.spki,
            name_constraints: None,
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    #[test]
    fn every_shipped_root_loads() {
        assert_eq!(
            super::anchors().len(),
            super::MICROSOFT.len() + super::PUBLIC.len()
        );
    }
    #[test]
    fn root_store_blobs_hold_the_certificate() {
        let cert: &[u8] = include_bytes!("../tests/data/trust/root.der");
        let mut blob = Vec::new();
        for (id, data) in [(3u32, &[1u8, 2, 3][..]), (32, cert)] {
            blob.extend_from_slice(&id.to_le_bytes());
            blob.extend_from_slice(&1u32.to_le_bytes());
            blob.extend_from_slice(&(data.len() as u32).to_le_bytes());
            blob.extend_from_slice(data);
        }
        assert_eq!(super::store_certificate(&blob), Some(cert));
        assert_eq!(super::store_certificate(&blob[..40]), None);
        let held = alloc::vec![cert.to_vec(), alloc::vec![1, 2, 3]];
        assert_eq!(super::of(&held).len(), 1);
    }
}
