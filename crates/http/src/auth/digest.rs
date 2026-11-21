//! HTTP Digest access authentication (RFC 7616, compatible with RFC 2617):
//! MD5, MD5-sess, SHA-256 and SHA-256-sess, qop "auth" (or none, the RFC
//! 2069 form). Challenges are parsed the way WinHTTP does: auth-params
//! with quoted strings and backslash escapes. `verify` is the server's
//! side, used by the test servers.
use super::md;
use alloc::{format, string::String, vec::Vec};

/// The auth-params of one challenge (`Digest realm="x", nonce=...`), lower
/// case names; None when it is not that scheme.
pub fn params(header: &str, scheme: &str) -> Option<Vec<(String, String)>> {
    let header = header.trim_start();
    let rest = header.get(scheme.len()..)?;
    if !header[..scheme.len()].eq_ignore_ascii_case(scheme)
        || !rest.starts_with([' ', '\t']) && !rest.is_empty()
    {
        return None;
    }
    let mut out = Vec::new();
    let mut chars = rest.chars().peekable();
    loop {
        while chars
            .peek()
            .is_some_and(|c| *c == ' ' || *c == '\t' || *c == ',')
        {
            chars.next();
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c == ',' || c == ' ' {
                break;
            }
            name.push(c.to_ascii_lowercase());
            chars.next();
        }
        if name.is_empty() {
            break;
        }
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        if chars.next() != Some('=') {
            return None;
        }
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            loop {
                match chars.next()? {
                    '\\' => value.push(chars.next()?),
                    '"' => break,
                    c => value.push(c),
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ',' || c == ' ' {
                    break;
                }
                value.push(c);
                chars.next();
            }
        }
        out.push((name, value));
    }
    Some(out)
}
pub fn param<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Md5,
    Md5Sess,
    Sha256,
    Sha256Sess,
}
impl Algorithm {
    fn parse(name: Option<&str>) -> Option<Self> {
        Some(match name.map(str::to_ascii_uppercase).as_deref() {
            None | Some("MD5") => Self::Md5,
            Some("MD5-SESS") => Self::Md5Sess,
            Some("SHA-256") => Self::Sha256,
            Some("SHA-256-SESS") => Self::Sha256Sess,
            _ => return None,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Md5Sess => "MD5-sess",
            Self::Sha256 => "SHA-256",
            Self::Sha256Sess => "SHA-256-sess",
        }
    }
    fn h(self, data: &str) -> String {
        match self {
            Self::Md5 | Self::Md5Sess => md::hex(&md::md5(data.as_bytes())),
            Self::Sha256 | Self::Sha256Sess => {
                use sha2::Digest;
                md::hex(&sha2::Sha256::digest(data.as_bytes()))
            }
        }
    }
    fn session(self) -> bool {
        matches!(self, Self::Md5Sess | Self::Sha256Sess)
    }
}

/// What the client sends for a request: the fields of the response.
pub struct Answer<'a> {
    pub user: &'a str,
    pub password: &'a str,
    pub method: &'a str,
    pub uri: &'a str,
    /// 16 random bytes for the client nonce.
    pub cnonce: [u8; 16],
    /// The nonce count (1 for the first use of a nonce).
    pub count: u32,
}
fn response(
    alg: Algorithm,
    a: &Answer,
    realm: &str,
    nonce: &str,
    qop: Option<&str>,
    cnonce: &str,
    nc: &str,
) -> String {
    let mut ha1 = alg.h(&format!("{}:{realm}:{}", a.user, a.password));
    if alg.session() {
        ha1 = alg.h(&format!("{ha1}:{nonce}:{cnonce}"));
    }
    let ha2 = alg.h(&format!("{}:{}", a.method, a.uri));
    match qop {
        Some(q) => alg.h(&format!("{ha1}:{nonce}:{nc}:{cnonce}:{q}:{ha2}")),
        None => alg.h(&format!("{ha1}:{nonce}:{ha2}")),
    }
}
/// The Authorization (or Proxy-Authorization) value answering `challenge`
/// (the whole `Digest ...` header value); None when it cannot be answered
/// (another scheme, an unknown algorithm, only qop=auth-int).
pub fn authorization(challenge: &str, a: &Answer) -> Option<String> {
    let p = params(challenge, "Digest")?;
    let realm = param(&p, "realm").unwrap_or("");
    let nonce = param(&p, "nonce")?;
    let alg = Algorithm::parse(param(&p, "algorithm"))?;
    let qop = match param(&p, "qop") {
        None => None,
        Some(list)
            if list
                .split(',')
                .any(|q| q.trim().eq_ignore_ascii_case("auth")) =>
        {
            Some("auth")
        }
        Some(_) => return None,
    };
    let cnonce = md::hex(&a.cnonce);
    let nc = format!("{:08x}", a.count);
    let digest = response(alg, a, realm, nonce, qop, &cnonce, &nc);
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut out = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{digest}\"",
        quote(a.user),
        quote(realm),
        quote(nonce),
        quote(a.uri)
    );
    if param(&p, "algorithm").is_some() {
        out.push_str(&format!(", algorithm={}", alg.name()));
    }
    if let Some(opaque) = param(&p, "opaque") {
        out.push_str(&format!(", opaque=\"{}\"", quote(opaque)));
    }
    if let Some(q) = qop {
        out.push_str(&format!(", qop={q}, nc={nc}, cnonce=\"{cnonce}\""));
    }
    Some(out)
}
/// Server side: does `authorization` answer the nonce for this user?
pub fn verify(authorization: &str, method: &str, password: &str, nonce: &str) -> bool {
    let Some(p) = params(authorization, "Digest") else {
        return false;
    };
    let (Some(user), Some(uri), Some(got), Some(n)) = (
        param(&p, "username"),
        param(&p, "uri"),
        param(&p, "response"),
        param(&p, "nonce"),
    ) else {
        return false;
    };
    let Some(alg) = Algorithm::parse(param(&p, "algorithm")) else {
        return false;
    };
    if n != nonce {
        return false;
    }
    let a = Answer {
        user,
        password,
        method,
        uri,
        cnonce: [0; 16],
        count: 0,
    };
    let expected = response(
        alg,
        &a,
        param(&p, "realm").unwrap_or(""),
        nonce,
        param(&p, "qop"),
        param(&p, "cnonce").unwrap_or(""),
        param(&p, "nc").unwrap_or(""),
    );
    expected.eq_ignore_ascii_case(got)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc2617_example_response() {
        // RFC 2617 §3.5: Mufasa / Circle Of Life.
        let challenge = "Digest realm=\"testrealm@host.com\", qop=\"auth,auth-int\", \
                         nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", \
                         opaque=\"5ccc069c403ebaf9f0171e9517f40e41\"";
        let p = params(challenge, "Digest").unwrap();
        let a = Answer {
            user: "Mufasa",
            password: "Circle Of Life",
            method: "GET",
            uri: "/dir/index.html",
            cnonce: [0; 16],
            count: 1,
        };
        let digest = response(
            Algorithm::Md5,
            &a,
            param(&p, "realm").unwrap(),
            param(&p, "nonce").unwrap(),
            Some("auth"),
            "0a4f113b",
            "00000001",
        );
        assert_eq!(digest, "6629fae49393a05397450978507c4ef1");
    }
    #[test]
    fn answers_verify_and_wrong_passwords_do_not() {
        let challenge = "Digest realm=\"doors\", nonce=\"abc123\", qop=\"auth\", algorithm=SHA-256";
        let a = Answer {
            user: "probe",
            password: "s3cret",
            method: "GET",
            uri: "/digest",
            cnonce: [5; 16],
            count: 1,
        };
        let header = authorization(challenge, &a).unwrap();
        assert!(verify(&header, "GET", "s3cret", "abc123"));
        assert!(!verify(&header, "GET", "wrong", "abc123"));
        assert!(!verify(&header, "POST", "s3cret", "abc123"));
        assert!(authorization("Basic realm=\"x\"", &a).is_none());
    }
}
