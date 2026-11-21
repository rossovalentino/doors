//! Security catalogs (`.cat`), as WinVerifyTrust's catalog check and the
//! CryptCATAdmin functions read them: a PKCS #7 SignedData whose content is
//! a certificate trust list (szOID_CTL 1.3.6.1.4.1.311.10.1, subject usage
//! szOID_CATALOG_LIST 1.3.6.1.4.1.311.12.1.1) naming each file it vouches
//! for by its hash. A member's identifier is the raw hash (version 2
//! catalogs: SHA-256 entries with SHA-1 entries beside them) or, in version
//! 1 catalogs, the hash as UTF-16 hexadecimal text; a member carrying an
//! SpcIndirectDataContent attribute also states its digest there. A
//! program's hash is its Authenticode image digest, any other file's the
//! hash of its bytes (CryptCATAdminCalcHashFromFileHandle). The catalog's
//! signer is checked like an embedded signature's.
use super::*;

const OID_CTL: &[u8] = &[0x2B, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x0A, 0x01];
const OID_CATALOG_LIST: &[u8] = &[0x2B, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x0C, 0x01, 0x01];
/// szOID_CATALOG_LIST_MEMBER2: the subject algorithm of version 2 catalogs.
const OID_CATALOG_MEMBER_V2: &[u8] = &[0x2B, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x0C, 0x01, 0x03];

/// A member: its tag (the name WinVerifyTrust's pcwszMemberTag gives: the
/// version 1 text, or the raw identifier in hexadecimal) and the hashes
/// it is named by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub tag: String,
    pub hashes: Vec<Vec<u8>>,
}
impl Member {
    pub fn matches(&self, hash: &[u8]) -> bool {
        self.hashes.iter().any(|h| h == hash)
    }
}
/// A catalog's members, as read before its signature is checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct List {
    /// 1, or 2 for a SHA-256 catalog (makecat's CatalogVersion=2).
    pub version: u32,
    pub members: Vec<Member>,
}
impl List {
    /// The member a tag names (letter case aside).
    pub fn by_tag(&self, tag: &str) -> Option<&Member> {
        self.members
            .iter()
            .find(|m| m.tag.eq_ignore_ascii_case(tag))
    }
    /// Whether some member is named by `hash`.
    pub fn contains(&self, hash: &[u8]) -> bool {
        self.members.iter().any(|m| m.matches(hash))
    }
    /// A file's hash as this catalog's members state it: SHA-256 in
    /// version 2, SHA-1 in version 1 (see `file_hashes`).
    pub fn hash_of(&self, file: &[u8]) -> Vec<u8> {
        let (sha1, sha256) = file_hashes(file);
        if self.version == 2 { sha256 } else { sha1 }
    }
}
/// A verified catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    pub signature: Signature,
    /// 1, or 2 for a SHA-256 catalog.
    pub version: u32,
    pub members: Vec<Member>,
    /// The list identifier (16 bytes makecat writes), as hexadecimal.
    pub identifier: String,
    /// ctlThisUpdate (Unix seconds).
    pub this_update: i64,
}
impl Catalog {
    /// Whether `hash` (a file's SHA-1 or SHA-256, `file_hashes`) is a member.
    pub fn contains(&self, hash: &[u8]) -> bool {
        self.members.iter().any(|m| m.matches(hash))
    }
}
/// The hashes a catalog names a file by: (SHA-1, SHA-256) of a program's
/// Authenticode image digest, or of any other file's bytes.
pub fn file_hashes(file: &[u8]) -> (Vec<u8>, Vec<u8>) {
    match layout(file) {
        Ok(l) => (
            image_digest(file, &l, Hash::Sha1),
            image_digest(file, &l, Hash::Sha256),
        ),
        Err(_) => (Hash::Sha1.digest(&[file]), Hash::Sha256.digest(&[file])),
    }
}
/// UTF-16LE hexadecimal text (a version 1 member tag) and its bytes.
fn tag_bytes(id: &[u8]) -> Option<(String, Vec<u8>)> {
    if !id.len().is_multiple_of(4) || id.is_empty() {
        return None;
    }
    let units: Vec<u16> = id
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let text = String::from_utf16(&units).ok()?;
    let text = text.trim_end_matches('\0');
    if text.is_empty()
        || !text.len().is_multiple_of(2)
        || !text.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    let bytes = (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    Some((String::from(text), bytes))
}
/// A member's hashes: its identifier (raw, or version 1 hexadecimal text)
/// and the digest of an SpcIndirectDataContent attribute.
fn member(subject: &[u8]) -> Option<Member> {
    let mut s = Reader::new(subject);
    let id = s.expect(der::OCTET_STRING)?.value;
    let mut hashes = Vec::new();
    let tag = if let Some((text, h)) = tag_bytes(id) {
        hashes.push(h);
        text
    } else {
        if matches!(id.len(), 20 | 32 | 48 | 64) {
            hashes.push(id.to_vec());
        }
        id.iter().map(|b| alloc::format!("{b:02X}")).collect()
    };
    if let Some(attrs) = s.optional(der::SET) {
        let mut list = Reader::new(attrs.value);
        while let Some(attr) = list.read() {
            let mut a = Reader::new(attr.value);
            if a.expect(der::OID).map(|o| o.value) != Some(OID_SPC_INDIRECT_DATA) {
                continue;
            }
            // SpcIndirectDataContent { data, DigestInfo { alg, digest } }.
            let Some(value) = a
                .expect(der::SET)
                .and_then(|set| Reader::new(set.value).read())
            else {
                continue;
            };
            let mut spc = Reader::new(value.value);
            spc.expect(der::SEQUENCE);
            if let Some(info) = spc.expect(der::SEQUENCE) {
                let mut info = Reader::new(info.value);
                info.expect(der::SEQUENCE);
                if let Some(d) = info.expect(der::OCTET_STRING)
                    && !hashes.iter().any(|h| h == d.value)
                {
                    hashes.push(d.value.to_vec());
                }
            }
        }
    }
    (!hashes.is_empty()).then_some(Member { tag, hashes })
}
/// The trust list of a catalog's SignedData: (list identifier, this
/// update, members).
fn trust_list(content: &[u8]) -> Result<(String, i64, List), Error> {
    // CertificateTrustList { version INTEGER OPTIONAL, subjectUsage
    // SEQUENCE OF OID, listIdentifier OCTET STRING OPTIONAL,
    // sequenceNumber INTEGER OPTIONAL, ctlThisUpdate Time, ctlNextUpdate
    // Time OPTIONAL, subjectAlgorithm AlgorithmIdentifier, trustedSubjects
    // SEQUENCE OF TrustedSubject OPTIONAL, ctlExtensions [0] OPTIONAL }.
    let mut ctl = Reader::new(content);
    ctl.optional(der::INTEGER);
    let usage = m(ctl.expect(der::SEQUENCE))?;
    if !Reader::new(usage.value)
        .read()
        .is_some_and(|o| o.tag == der::OID && o.value == OID_CATALOG_LIST)
    {
        return Err(Error::Malformed);
    }
    let identifier = ctl
        .optional(der::OCTET_STRING)
        .map(|t| t.value.iter().map(|b| alloc::format!("{b:02X}")).collect())
        .unwrap_or_default();
    ctl.optional(der::INTEGER);
    let this_update = m(ctl.read().and_then(der::time))?;
    if matches!(ctl.peek_tag(), Some(der::UTC_TIME | der::GENERALIZED_TIME)) {
        ctl.read();
    }
    let algorithm = m(ctl.expect(der::SEQUENCE))?;
    let version = if Reader::new(algorithm.value)
        .expect(der::OID)
        .is_some_and(|o| o.value == OID_CATALOG_MEMBER_V2)
    {
        2
    } else {
        1
    };
    let mut members = Vec::new();
    if let Some(subjects) = ctl.optional(der::SEQUENCE) {
        let mut list = Reader::new(subjects.value);
        while let Some(subject) = list.read() {
            members.extend(member(subject.value));
        }
    }
    Ok((identifier, this_update, List { version, members }))
}
/// A catalog's members, unverified (to find the catalogs naming a hash,
/// as the catalog database does, and to check a member before the
/// signature, as WinVerifyTrust orders its checks).
pub fn read(cat: &[u8]) -> Option<List> {
    let sd = signed_data(cat).ok()?;
    if sd.content_type != OID_CTL || sd.content.tag != der::SEQUENCE {
        return None;
    }
    trust_list(sd.content.value).ok().map(|(_, _, list)| list)
}
/// Verify catalog `cat` at Unix time `now` (its signer as an embedded
/// signature's, with revocation when `revocation` is given) and read its
/// members.
pub fn verify(
    cat: &[u8],
    now: i64,
    anchors: &[Anchor<'_>],
    revocation: Option<&mut dyn revocation::Source>,
) -> Result<Catalog, Error> {
    let sd = signed_data(cat)?;
    if sd.content_type != OID_CTL || sd.content.tag != der::SEQUENCE {
        return Err(Error::Malformed);
    }
    let (identifier, this_update, list) = trust_list(sd.content.value)?;
    let signature = verify_signer(
        &sd,
        OID_CTL,
        sd.signer.digest.name(),
        now,
        anchors,
        revocation,
    )?;
    Ok(Catalog {
        signature,
        version: list.version,
        members: list.members,
        identifier,
        this_update,
    })
}
