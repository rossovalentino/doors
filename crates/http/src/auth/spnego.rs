//! SPNEGO (RFC 4178) as HTTP Negotiate uses it (RFC 4559), with NTLM as
//! the only mechanism: Doors has no Kerberos realm to offer. Tokens are
//! DER; only the fields this exchange needs are read.
use alloc::vec::Vec;

/// 1.3.6.1.5.5.2
const SPNEGO: [u8; 6] = [0x2b, 0x06, 0x01, 0x05, 0x05, 0x02];
/// 1.3.6.1.4.1.311.2.2.10 (NTLMSSP)
const NTLMSSP: [u8; 10] = [0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x02, 0x02, 0x0a];

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = alloc::vec![tag];
    let n = content.len();
    if n < 0x80 {
        out.push(n as u8);
    } else if n < 0x100 {
        out.extend_from_slice(&[0x81, n as u8]);
    } else {
        out.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]);
    }
    out.extend_from_slice(content);
    out
}
/// One element: (tag, content, rest).
fn element(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *b.first()?;
    let first = *b.get(1)?;
    let (len, at) = match first {
        n if n < 0x80 => (usize::from(n), 2),
        0x81 => (usize::from(*b.get(2)?), 3),
        0x82 => ((usize::from(*b.get(2)?) << 8) | usize::from(*b.get(3)?), 4),
        _ => return None,
    };
    let content = b.get(at..at + len)?;
    Some((tag, content, &b[at + len..]))
}
/// The context-tagged fields of a SEQUENCE: (tag number, content).
fn fields(sequence: &[u8]) -> Option<Vec<(u8, &[u8])>> {
    let (tag, mut rest, _) = element(sequence)?;
    if tag != 0x30 {
        return None;
    }
    let mut out = Vec::new();
    while !rest.is_empty() {
        let (tag, content, next) = element(rest)?;
        out.push((tag & 0x1f, content));
        rest = next;
    }
    Some(out)
}

/// NegTokenInit offering NTLM with its first token.
pub fn init(ntlm: &[u8]) -> Vec<u8> {
    let mechs = tlv(0x30, &tlv(0x06, &NTLMSSP));
    let mut body = tlv(0xa0, &mechs);
    body.extend_from_slice(&tlv(0xa2, &tlv(0x04, ntlm)));
    let mut inner = tlv(0x06, &SPNEGO);
    inner.extend_from_slice(&tlv(0xa0, &tlv(0x30, &body)));
    tlv(0x60, &inner)
}
/// NegTokenResp carrying `token` (the client's next leg, or the server's
/// answer with `state`: 0 accept-completed, 1 accept-incomplete).
pub fn response(state: Option<u8>, token: Option<&[u8]>) -> Vec<u8> {
    let mut body = Vec::new();
    if let Some(s) = state {
        body.extend_from_slice(&tlv(0xa0, &tlv(0x0a, &[s])));
        body.extend_from_slice(&tlv(0xa1, &tlv(0x06, &NTLMSSP)));
    }
    if let Some(t) = token {
        body.extend_from_slice(&tlv(0xa2, &tlv(0x04, t)));
    }
    tlv(0xa1, &tlv(0x30, &body))
}
/// The NTLM token inside a NegTokenResp (or a bare NTLM token, which some
/// servers send under the Negotiate scheme).
pub fn response_token(token: &[u8]) -> Option<Vec<u8>> {
    if token.starts_with(b"NTLMSSP\0") {
        return Some(token.to_vec());
    }
    let (tag, content, _) = element(token)?;
    if tag != 0xa1 {
        return None;
    }
    let f = fields(content)?;
    let (_, wrapped) = f.iter().find(|(n, _)| *n == 2)?;
    let (tag, inner, _) = element(wrapped)?;
    (tag == 0x04).then(|| inner.to_vec())
}
/// Server side: the mechanism token of a NegTokenInit (or a bare NTLM
/// token).
pub fn init_token(token: &[u8]) -> Option<Vec<u8>> {
    if token.starts_with(b"NTLMSSP\0") {
        return Some(token.to_vec());
    }
    let (tag, content, _) = element(token)?;
    if tag != 0x60 {
        return response_token(token);
    }
    let (oid_tag, oid, rest) = element(content)?;
    if oid_tag != 0x06 || oid != SPNEGO {
        return None;
    }
    let (tag, init, _) = element(rest)?;
    if tag != 0xa0 {
        return None;
    }
    let f = fields(init)?;
    let (_, wrapped) = f.iter().find(|(n, _)| *n == 2)?;
    let (tag, inner, _) = element(wrapped)?;
    (tag == 0x04).then(|| inner.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_round_trip() {
        let ntlm = super::super::ntlm::negotiate();
        assert_eq!(init_token(&init(&ntlm)).unwrap(), ntlm);
        let reply = response(Some(1), Some(b"NTLMSSP\0challenge"));
        assert_eq!(response_token(&reply).unwrap(), b"NTLMSSP\0challenge");
        let last = response(None, Some(&[7u8; 300]));
        assert_eq!(init_token(&last).unwrap(), [7u8; 300]);
        assert_eq!(response_token(b"NTLMSSP\0raw").unwrap(), b"NTLMSSP\0raw");
    }
}
