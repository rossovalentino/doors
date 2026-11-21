//! NTLM authentication (MS-NLMP) for HTTP: NTLMv2 responses with extended
//! session security, and the MIC when the server's target information has
//! a timestamp. Only authentication: no signing or sealing keys are made
//! (HTTP does not use them), so the negotiate message asks for neither.
//! The `server_*` functions are the test servers' side.
use super::md::{hmac_md5, md4};
use alloc::{string::String, vec::Vec};

const SIGNATURE: &[u8; 8] = b"NTLMSSP\0";
const UNICODE: u32 = 0x1;
const OEM: u32 = 0x2;
const REQUEST_TARGET: u32 = 0x4;
const NTLM: u32 = 0x200;
const ALWAYS_SIGN: u32 = 0x8000;
const EXTENDED_SESSION_SECURITY: u32 = 0x8_0000;
const TARGET_INFO: u32 = 0x80_0000;
const VERSION: u32 = 0x200_0000;
const N128: u32 = 0x2000_0000;
const N56: u32 = 0x8000_0000;
pub const FLAGS: u32 = UNICODE
    | OEM
    | REQUEST_TARGET
    | NTLM
    | ALWAYS_SIGN
    | EXTENDED_SESSION_SECURITY
    | VERSION
    | N128
    | N56;
/// Windows 10/11 product version 10.0, build 19041, NTLM revision 15.
const PRODUCT: [u8; 8] = [10, 0, 0x61, 0x4a, 0, 0, 0, 15];

const AV_EOL: u16 = 0;
const AV_TIMESTAMP: u16 = 7;
const AV_FLAGS: u16 = 6;

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}
fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
/// A (length, max length, offset) field's bytes.
fn field(b: &[u8], at: usize) -> Option<&[u8]> {
    let len = usize::from(u16_at(b, at)?);
    let offset = u32_at(b, at + 4)? as usize;
    b.get(offset..offset.checked_add(len)?)
}
fn put_field(header: &mut [u8], at: usize, payload: &mut Vec<u8>, base: usize, data: &[u8]) {
    let offset = (base + payload.len()) as u32;
    header[at..at + 2].copy_from_slice(&(data.len() as u16).to_le_bytes());
    header[at + 2..at + 4].copy_from_slice(&(data.len() as u16).to_le_bytes());
    header[at + 4..at + 8].copy_from_slice(&offset.to_le_bytes());
    payload.extend_from_slice(data);
}

/// The NEGOTIATE_MESSAGE (no domain or workstation supplied).
pub fn negotiate() -> Vec<u8> {
    let mut m = Vec::with_capacity(40);
    m.extend_from_slice(SIGNATURE);
    m.extend_from_slice(&1u32.to_le_bytes());
    m.extend_from_slice(&FLAGS.to_le_bytes());
    m.extend_from_slice(&[0; 16]);
    m.extend_from_slice(&PRODUCT);
    m
}

/// A parsed CHALLENGE_MESSAGE.
pub struct Challenge<'a> {
    pub flags: u32,
    pub server: [u8; 8],
    pub target_info: &'a [u8],
}
pub fn parse_challenge(m: &[u8]) -> Option<Challenge<'_>> {
    if m.get(..8)? != SIGNATURE || u32_at(m, 8)? != 2 {
        return None;
    }
    let flags = u32_at(m, 20)?;
    let server = m.get(24..32)?.try_into().ok()?;
    let target_info = if flags & TARGET_INFO != 0 {
        field(m, 40)?
    } else {
        &[]
    };
    Some(Challenge {
        flags,
        server,
        target_info,
    })
}
/// The AV pairs of target information: (id, value), without the EOL.
fn pairs(info: &[u8]) -> Option<Vec<(u16, &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0;
    loop {
        let id = u16_at(info, at)?;
        let len = usize::from(u16_at(info, at + 2)?);
        if id == AV_EOL {
            return Some(out);
        }
        out.push((id, info.get(at + 4..at + 4 + len)?));
        at += 4 + len;
    }
}

/// NTOWFv2: HMAC-MD5 of the password's MD4 over the upper-case user name
/// and the domain, both UTF-16LE.
pub fn ntowf_v2(user: &str, domain: &str, password: &str) -> [u8; 16] {
    let mut identity = utf16(&user.to_uppercase());
    identity.extend_from_slice(&utf16(domain));
    hmac_md5(&md4(&utf16(password)), &[&identity])
}

/// What an AUTHENTICATE_MESSAGE is made from.
pub struct Identity<'a> {
    pub user: &'a str,
    pub domain: &'a str,
    pub password: &'a str,
    pub workstation: &'a str,
}
/// The AUTHENTICATE_MESSAGE answering `challenge`, given the negotiate
/// message sent (for the MIC), 8 random bytes for the client challenge and
/// the time as a FILETIME (used when the server sent no timestamp).
pub fn authenticate(
    negotiate: &[u8],
    challenge: &[u8],
    id: &Identity,
    client: [u8; 8],
    now: u64,
) -> Option<Vec<u8>> {
    let c = parse_challenge(challenge)?;
    let mut av = pairs(c.target_info)?;
    let timestamp = av
        .iter()
        .find(|(k, _)| *k == AV_TIMESTAMP)
        .and_then(|(_, v)| Some(u64::from_le_bytes((*v).try_into().ok()?)));
    let mic = timestamp.is_some();
    // With a timestamp the MIC is sent, and MsvAvFlags says so.
    let flags_value = 2u32.to_le_bytes();
    if mic {
        av.retain(|(k, _)| *k != AV_FLAGS);
        av.push((AV_FLAGS, &flags_value));
    }
    let mut info = Vec::new();
    for (k, v) in &av {
        info.extend_from_slice(&k.to_le_bytes());
        info.extend_from_slice(&(v.len() as u16).to_le_bytes());
        info.extend_from_slice(v);
    }
    info.extend_from_slice(&[0; 4]);
    let key = ntowf_v2(id.user, id.domain, id.password);
    let mut temp = alloc::vec![1u8, 1, 0, 0, 0, 0, 0, 0];
    temp.extend_from_slice(&timestamp.unwrap_or(now).to_le_bytes());
    temp.extend_from_slice(&client);
    temp.extend_from_slice(&[0; 4]);
    temp.extend_from_slice(&info);
    temp.extend_from_slice(&[0; 4]);
    let proof = hmac_md5(&key, &[&c.server, &temp]);
    let mut nt = proof.to_vec();
    nt.extend_from_slice(&temp);
    let lm = if mic {
        alloc::vec![0u8; 24]
    } else {
        let mut lm = hmac_md5(&key, &[&c.server, &client]).to_vec();
        lm.extend_from_slice(&client);
        lm
    };
    let session = hmac_md5(&key, &[&proof]);
    let flags = (FLAGS & c.flags) | UNICODE;
    let mut header = alloc::vec![0u8; 88];
    header[..8].copy_from_slice(SIGNATURE);
    header[8..12].copy_from_slice(&3u32.to_le_bytes());
    header[60..64].copy_from_slice(&flags.to_le_bytes());
    header[64..72].copy_from_slice(&PRODUCT);
    let mut payload = Vec::new();
    put_field(&mut header, 12, &mut payload, 88, &lm);
    put_field(&mut header, 20, &mut payload, 88, &nt);
    put_field(&mut header, 28, &mut payload, 88, &utf16(id.domain));
    put_field(&mut header, 36, &mut payload, 88, &utf16(id.user));
    put_field(&mut header, 44, &mut payload, 88, &utf16(id.workstation));
    put_field(&mut header, 52, &mut payload, 88, &[]);
    header.extend_from_slice(&payload);
    if mic {
        let code = hmac_md5(&session, &[negotiate, challenge, &header]);
        header[72..88].copy_from_slice(&code);
    }
    Some(header)
}

/// "DOMAIN\user" or "user@domain" split as Windows does; a plain name has
/// no domain.
pub fn split_user(name: &str) -> (String, String) {
    if let Some((domain, user)) = name.split_once('\\') {
        return (user.into(), domain.into());
    }
    if let Some((user, domain)) = name.rsplit_once('@') {
        return (user.into(), domain.into());
    }
    (name.into(), String::new())
}

/// The NEGOTIATE_MESSAGE's flags.
pub fn negotiate_flags(m: &[u8]) -> Option<u32> {
    if m.get(..8)? != SIGNATURE || u32_at(m, 8)? != 1 {
        return None;
    }
    u32_at(m, 12)
}
/// Server side: a CHALLENGE_MESSAGE with target information naming
/// `domain` and carrying `timestamp`, answering a negotiate message's
/// `requested` flags as Windows servers do (Unicode chosen over OEM).
pub fn server_challenge(server: [u8; 8], domain: &str, timestamp: u64, requested: u32) -> Vec<u8> {
    let name = utf16(domain);
    let mut info = Vec::new();
    for (id, value) in [
        (2u16, &name[..]),
        (1, &name[..]),
        (7, &timestamp.to_le_bytes()[..]),
    ] {
        info.extend_from_slice(&id.to_le_bytes());
        info.extend_from_slice(&(value.len() as u16).to_le_bytes());
        info.extend_from_slice(value);
    }
    info.extend_from_slice(&[0; 4]);
    let mut header = alloc::vec![0u8; 56];
    header[..8].copy_from_slice(SIGNATURE);
    header[8..12].copy_from_slice(&2u32.to_le_bytes());
    const SIGN: u32 = 0x10;
    const SEAL: u32 = 0x20;
    const KEY_EXCH: u32 = 0x4000_0000;
    const TARGET_TYPE_DOMAIN: u32 = 0x1_0000;
    let echoed = requested
        & (NTLM
            | ALWAYS_SIGN
            | EXTENDED_SESSION_SECURITY
            | VERSION
            | N128
            | N56
            | SIGN
            | SEAL
            | KEY_EXCH);
    let charset = if requested & UNICODE != 0 {
        UNICODE
    } else {
        OEM
    };
    let flags = echoed | charset | REQUEST_TARGET | TARGET_INFO | TARGET_TYPE_DOMAIN;
    header[20..24].copy_from_slice(&flags.to_le_bytes());
    header[24..32].copy_from_slice(&server);
    header[48..56].copy_from_slice(&PRODUCT);
    let mut payload = Vec::new();
    put_field(&mut header, 12, &mut payload, 56, &name);
    put_field(&mut header, 40, &mut payload, 56, &info);
    header.extend_from_slice(&payload);
    header
}
/// Server side: the user an AUTHENTICATE_MESSAGE proves, when its NTLMv2
/// response matches `password` for the `server` challenge.
pub fn server_verify(m: &[u8], server: [u8; 8], password: &str) -> Option<String> {
    if m.get(..8)? != SIGNATURE || u32_at(m, 8)? != 3 {
        return None;
    }
    let nt = field(m, 20)?;
    let decode = |b: &[u8]| -> Option<String> {
        let units: Vec<u16> = b
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&units).ok()
    };
    let domain = decode(field(m, 28)?)?;
    let user = decode(field(m, 36)?)?;
    if nt.len() < 16 + 28 {
        return None;
    }
    let key = ntowf_v2(&user, &domain, password);
    let proof = hmac_md5(&key, &[&server, &nt[16..]]);
    (proof[..] == nt[..16]).then_some(user)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ntowf_v2_matches_ms_nlmp_4_2_4() {
        // MS-NLMP 4.2.4.1.1: User / Domain / Password.
        let key = ntowf_v2("User", "Domain", "Password");
        assert_eq!(
            key,
            [
                0x0c, 0x86, 0x8a, 0x40, 0x3b, 0xfd, 0x7a, 0x93, 0xa3, 0x00, 0x1e, 0xf2, 0x2e, 0xf0,
                0x2e, 0x3f
            ]
        );
    }
    #[test]
    fn the_server_accepts_our_answer_and_refuses_a_wrong_password() {
        let server = [1, 2, 3, 4, 5, 6, 7, 8];
        let challenge = server_challenge(server, "DOORS", 0x01d9_0000_0000_0000, FLAGS);
        let negotiate = negotiate();
        let id = Identity {
            user: "probe",
            domain: "DOORS",
            password: "s3cret",
            workstation: "DOORS-PC",
        };
        let message = authenticate(&negotiate, &challenge, &id, [9; 8], 0).unwrap();
        assert_eq!(
            server_verify(&message, server, "s3cret").as_deref(),
            Some("probe")
        );
        assert_eq!(server_verify(&message, server, "other"), None);
        // A timestamp in the challenge: the MIC is filled in.
        assert_ne!(&message[72..88], &[0; 16]);
    }
    #[test]
    fn user_names_split_like_windows() {
        assert_eq!(split_user("DOORS\\probe"), ("probe".into(), "DOORS".into()));
        assert_eq!(
            split_user("probe@doors.test"),
            ("probe".into(), "doors.test".into())
        );
        assert_eq!(split_user("probe"), ("probe".into(), String::new()));
    }
}
