//! Fixtures from `tests/data/generate.js` (a tiny PE32+ signed per the
//! Authenticode format with an OpenSSL test PKI; Windows' own
//! `Get-AuthenticodeSignature` reports `signed.exe` as hash-valid with an
//! untrusted root and `tampered.exe` as HashMismatch).
use super::*;
use door_tls::verify::Anchor;

const ROOT: &[u8] = include_bytes!("../tests/data/root.der");
const OTHER: &[u8] = include_bytes!("../tests/data/other-root.der");
/// 2026-11-02: the fixtures' certificates have fixed validity periods
/// (generate.js); only the 2024 signer has expired by then.
const NOW: i64 = 1_793_577_600;
/// 2024-06-15 12:00 UTC, the time-stamping authority's time in timestamped.exe.
const STAMPED: i64 = 1_718_452_800;
fn anchor(der: &'static [u8]) -> Anchor<'static> {
    let c = door_tls::x509::parse(der).unwrap();
    Anchor {
        subject: c.subject,
        spki: c.spki,
        name_constraints: None,
    }
}
#[test]
fn a_valid_signature_names_its_publisher() {
    let s = verify(
        include_bytes!("../tests/data/signed.exe"),
        NOW,
        &[anchor(ROOT)],
    )
    .unwrap();
    assert_eq!(s.publisher, "Doors Test Publisher");
    assert_eq!(s.issuer, "Doors Test Root");
    assert_eq!(s.digest, "SHA256");
    assert_eq!(s.timestamp, None);
}
#[test]
fn tampering_trust_time_and_usage_are_each_refused() {
    let roots = [anchor(ROOT)];
    assert_eq!(
        verify(include_bytes!("../tests/data/tampered.exe"), NOW, &roots),
        Err(Error::Modified)
    );
    assert_eq!(
        verify(include_bytes!("../tests/data/unsigned.exe"), NOW, &roots),
        Err(Error::Unsigned)
    );
    assert_eq!(
        verify(
            include_bytes!("../tests/data/signed.exe"),
            NOW,
            &[anchor(OTHER)]
        ),
        Err(Error::Untrusted)
    );
    assert_eq!(
        verify(include_bytes!("../tests/data/expired.exe"), NOW, &roots),
        Err(Error::Expired)
    );
    assert_eq!(
        verify(include_bytes!("../tests/data/wrong-usage.exe"), NOW, &roots),
        Err(Error::WrongUsage)
    );
    assert_eq!(verify(b"not a program", NOW, &roots), Err(Error::NotPe));
}
#[test]
fn bytes_after_the_certificate_table_or_in_its_padding_are_refused() {
    let roots = [anchor(ROOT)];
    let mut appended = include_bytes!("../tests/data/signed.exe").to_vec();
    appended.extend_from_slice(b"payload smuggled after the signature");
    assert_eq!(verify(&appended, NOW, &roots), Err(Error::Malformed));
    // A flipped byte inside the signature blob's zero padding.
    let mut padded = include_bytes!("../tests/data/signed.exe").to_vec();
    let last = padded.len() - 1;
    if padded[last] == 0 {
        padded[last] = 1;
        assert!(verify(&padded, NOW, &roots).is_err());
    }
}
#[test]
fn a_trusted_timestamp_keeps_an_expired_signature_valid() {
    let roots = [anchor(ROOT)];
    let s = verify(include_bytes!("../tests/data/timestamped.exe"), NOW, &roots).unwrap();
    assert_eq!(s.publisher, "Doors Expired Publisher");
    assert_eq!(s.timestamp, Some(STAMPED));
    // Stamped after the certificate expired: still expired.
    assert_eq!(
        verify(
            include_bytes!("../tests/data/late-timestamp.exe"),
            NOW,
            &roots
        ),
        Err(Error::Expired)
    );
    // A stamp from an untrusted authority counts for nothing.
    assert_eq!(
        verify(
            include_bytes!("../tests/data/untrusted-timestamp.exe"),
            NOW,
            &roots
        ),
        Err(Error::Expired)
    );
    // Under a foreign root neither the stamp nor the signer is trusted.
    assert!(
        verify(
            include_bytes!("../tests/data/timestamped.exe"),
            NOW,
            &[anchor(OTHER)]
        )
        .is_err()
    );
}
#[test]
fn display_names_follow_simple_display_order() {
    let c = door_tls::x509::parse(ROOT).unwrap();
    assert_eq!(display_name(c.subject), "Doors Test Root");
}
/// Real signed programs on a Windows PC: DOOR_AUTHENTICODE_LIVE lists
/// `path=Expected Publisher` pairs separated by `;` (`!` instead of a
/// publisher expects a failure). Run with `--ignored`.
#[test]
#[ignore]
fn live_programs_verify_like_windows() {
    let Ok(list) = std::env::var("DOOR_AUTHENTICODE_LIVE") else {
        return;
    };
    let mut roots = door_tls::verify::mozilla_roots();
    roots.extend(super::roots::anchors());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    for item in list.split(';').filter(|s| !s.is_empty()) {
        let (path, want) = item.rsplit_once('=').unwrap();
        let bytes = std::fs::read(path).unwrap();
        let got = verify(&bytes, now, &roots);
        std::println!("{path}: {got:?}");
        if want == "!" {
            assert!(got.is_err(), "{path}");
        } else {
            assert_eq!(got.map(|s| s.publisher).as_deref(), Ok(want), "{path}");
        }
    }
}

/// Catalog and revocation fixtures (`tests/data/trust`, see its
/// `generate.md`): a test root, a catalog publisher whose certificate names
/// the CRL at `TRUST_CRL_URL`, a second publisher the CRL revokes from
/// 2026-03-01, programs each signed by one of them (signtool) and a version
/// 2 catalog (makecat, SHA-256) of `unsigned.exe` and `readme.txt` signed by
/// the first.
mod trust {
    use super::*;
    const TRUST_ROOT: &[u8] = include_bytes!("../tests/data/trust/root.der");
    const PUBLISHER: &[u8] = include_bytes!("../tests/data/trust/publisher.der");
    const GOOD: &[u8] = include_bytes!("../tests/data/trust/signed-good.exe");
    const REVOKED: &[u8] = include_bytes!("../tests/data/trust/signed-revoked.exe");
    const CAT: &[u8] = include_bytes!("../tests/data/trust/doors-trust-test.cat");
    const CRL: &[u8] = include_bytes!("../tests/data/trust/doors-trust-test.crl");
    const README: &[u8] = include_bytes!("../tests/data/trust/readme.txt");
    const HELLO: &[u8] = include_bytes!("../tests/data/unsigned.exe");
    const TRUST_CRL_URL: &str = "http://10.0.2.2:28480/doors-trust-test.crl";
    /// The CRL's thisUpdate and nextUpdate: 2026-01-01 and 2036-01-01.
    const CRL_ISSUED: i64 = 1_767_225_600;
    const CRL_EXPIRES: i64 = 2_082_758_400;
    /// One CRL for every URL (or none: offline), counting fetches.
    struct Fixed(Option<&'static [u8]>, usize);
    impl revocation::Source for Fixed {
        fn crl(&mut self, url: &str) -> Option<Vec<u8>> {
            assert_eq!(url, TRUST_CRL_URL);
            self.1 += 1;
            self.0.map(<[u8]>::to_vec)
        }
    }
    #[test]
    fn crls_revoke_publishers_and_offline_checks_fail() {
        let roots = [anchor(TRUST_ROOT)];
        assert_eq!(
            revocation::distribution_points(PUBLISHER),
            alloc::vec![String::from(TRUST_CRL_URL)]
        );
        assert!(revocation::distribution_points(TRUST_ROOT).is_empty());
        let crl = revocation::parse(CRL).unwrap();
        assert_eq!(crl.this_update(), CRL_ISSUED);
        assert_eq!(crl.next_update(), Some(CRL_EXPIRES));
        // Serial 0x5012, revoked 2026-03-01.
        assert_eq!(crl.revoked(&[0x50, 0x12]), Some(1_772_323_200));
        assert_eq!(crl.revoked(&[0x00, 0x50, 0x12]), Some(1_772_323_200));
        assert_eq!(crl.revoked(&[0x50, 0x11]), None);
        // Unchecked, both signatures are good.
        assert_eq!(
            verify(GOOD, NOW, &roots).map(|s| s.publisher).as_deref(),
            Ok("Doors Catalog Publisher")
        );
        assert_eq!(
            verify(REVOKED, NOW, &roots).map(|s| s.publisher).as_deref(),
            Ok("Doors Revoked Publisher")
        );
        let mut online = Fixed(Some(CRL), 0);
        assert!(verify_full(GOOD, NOW, &roots, Some(&mut online)).is_ok());
        assert_eq!(online.1, 1);
        assert_eq!(
            verify_full(REVOKED, NOW, &roots, Some(&mut online)),
            Err(Error::Revoked)
        );
        assert_eq!(Error::Revoked.hresult(), 0x800B_010C);
        // No CRL, or only a stale one (the check is at 2036-06-01): a
        // revocation failure, never a pass.
        let mut offline = Fixed(None, 0);
        assert_eq!(
            verify_full(GOOD, NOW, &roots, Some(&mut offline)),
            Err(Error::RevocationOffline)
        );
        assert_eq!(Error::RevocationOffline.hresult(), 0x800B_010E);
        let mut stale = Fixed(Some(CRL), 0);
        assert!(matches!(
            verify_full(GOOD, CRL_EXPIRES + 151 * 86_400, &roots, Some(&mut stale)),
            Err(Error::RevocationOffline | Error::Expired)
        ));
        // A CRL signed by another key is no answer.
        let mut forged = CRL.to_vec();
        let at = forged.len() - 10;
        forged[at] ^= 1;
        let forged: &'static [u8] = alloc::boxed::Box::leak(forged.into_boxed_slice());
        let mut wrong = Fixed(Some(forged), 0);
        assert_eq!(
            verify_full(REVOKED, NOW, &roots, Some(&mut wrong)),
            Err(Error::RevocationOffline)
        );
    }
    #[test]
    fn catalogs_vouch_for_files_by_hash() {
        let roots = [anchor(TRUST_ROOT)];
        let c = catalog::verify(CAT, NOW, &roots, None).unwrap();
        assert_eq!(c.signature.publisher, "Doors Catalog Publisher");
        assert_eq!(c.signature.issuer, "Doors Trust Test Root");
        assert_eq!(c.signature.digest, "SHA256");
        assert_eq!(c.identifier, "A1DA48AC8AAA9E4395DE2E6AB24917F3");
        // 2026-11-09 07:22:36 UTC.
        assert_eq!(c.this_update, 1_794_208_956);
        // SHA-256 entries for both files, SHA-1 entries beside them.
        assert_eq!(c.members.len(), 4);
        assert_eq!(
            c.members[0].tag,
            "03879F52657095D5666DFE22A8EBA63FCB8EFA3AB2102875FC73D7C0F0C69137"
        );
        assert_eq!(c.members[2].tag, "A4E574683D76883C90B2718C4C3CE58BCD93F71C");
        for file in [HELLO, README] {
            let (sha1, sha256) = catalog::file_hashes(file);
            assert_eq!((sha1.len(), sha256.len()), (20, 32));
            assert!(c.contains(&sha1) && c.contains(&sha256));
        }
        // The image digest leaves the signature out: a signed copy of a
        // member is a member; other files are not.
        assert_eq!(catalog::file_hashes(GOOD), catalog::file_hashes(HELLO));
        let (sha1, sha256) = catalog::file_hashes(CRL);
        assert!(!c.contains(&sha1) && !c.contains(&sha256));
        // A program's member hash is its Authenticode image digest.
        assert_ne!(catalog::file_hashes(HELLO).1, Hash::Sha256.digest(&[HELLO]));
        assert_eq!(c.version, 2);
        let list = catalog::read(CAT).unwrap();
        assert_eq!((list.version, list.members.len()), (2, 4));
        let tag = "03879f52657095d5666dfe22a8eba63fcb8efa3ab2102875fc73d7c0f0c69137";
        let hello = list.by_tag(tag).unwrap();
        assert!(hello.matches(&list.hash_of(HELLO)));
        assert!(!hello.matches(&list.hash_of(README)));
        // A SHA-1 entry is no match for a version 2 catalog's SHA-256.
        let sha1 = list
            .by_tag("B9F5ACC0ABC955361BB383561FEEC21DFC8131D2")
            .unwrap();
        assert!(!sha1.matches(&list.hash_of(HELLO)));
        assert!(list.by_tag("00112233").is_none());
        assert!(catalog::read(README).is_none());
        // With revocation, against another root, and tampered.
        let mut online = Fixed(Some(CRL), 0);
        assert!(catalog::verify(CAT, NOW, &roots, Some(&mut online)).is_ok());
        // The catalog carries no root: no chain at all (CERT_E_CHAINING,
        // as Windows reports it), not an untrusted root.
        assert_eq!(
            catalog::verify(CAT, NOW, &[anchor(OTHER)], None),
            Err(Error::Chaining)
        );
        assert_eq!(
            verify(GOOD, NOW, &[anchor(OTHER)]).map_err(Error::hresult),
            Err(0x800B_010A)
        );
        let mut bad = CAT.to_vec();
        bad[140] ^= 1;
        assert_eq!(
            catalog::verify(&bad, NOW, &roots, None),
            Err(Error::Modified)
        );
        assert_eq!(
            catalog::verify(GOOD, NOW, &roots, None),
            Err(Error::Malformed)
        );
    }
}
