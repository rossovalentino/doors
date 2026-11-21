//! HTTP authentication for WinHTTP and WinINet: Basic, Digest (RFC 7616),
//! NTLM (MS-NLMP) and Negotiate (RFC 4559: NTLM, bare as Windows sends it
//! without Kerberos, or in SPNEGO when the server answers that way). `Client`
//! answers the challenges of a 401 or 407 response with the header to send
//! next; the NTLM legs run on one connection.
pub mod digest;
pub mod md;
pub mod ntlm;
pub mod spnego;

use alloc::{format, string::String, vec::Vec};

/// The WINHTTP_AUTH_SCHEME_* bits (WinINet uses the same names).
pub const BASIC: u32 = 0x1;
pub const NTLM: u32 = 0x2;
pub const PASSPORT: u32 = 0x4;
pub const DIGEST: u32 = 0x8;
pub const NEGOTIATE: u32 = 0x10;

/// The scheme bit a challenge header names.
pub fn scheme_of(challenge: &str) -> u32 {
    match challenge
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "basic" => BASIC,
        "ntlm" => NTLM,
        "passport1.4" => PASSPORT,
        "digest" => DIGEST,
        "negotiate" => NEGOTIATE,
        _ => 0,
    }
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub user: String,
    pub password: String,
    /// The schemes these credentials may answer (WinHTTP names one;
    /// WinINet allows any).
    pub schemes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Fresh,
    /// The NTLM negotiate message went out (inside SPNEGO or not).
    Ntlm {
        negotiate: Vec<u8>,
        spnego: bool,
    },
    Answered,
}
/// One request's side of an authentication exchange with a server or a
/// proxy.
#[derive(Clone, Debug)]
pub struct Client {
    pub credentials: Option<Credentials>,
    state: State,
    /// The challenges of the last 401 (or 407) seen, for an answer after
    /// the application sets credentials (WinHttpSetCredentials).
    pub last: Vec<String>,
}
impl Default for Client {
    fn default() -> Self {
        Self {
            credentials: None,
            state: State::Fresh,
            last: Vec::new(),
        }
    }
}
/// What an answer needs from the request and the system.
pub struct Request<'a> {
    pub method: &'a str,
    pub uri: &'a str,
    /// 16 random bytes (Digest's cnonce) and 8 more (NTLM's client
    /// challenge).
    pub random: [u8; 24],
    /// The time as a FILETIME.
    pub now: u64,
    pub workstation: &'a str,
}
impl Client {
    /// A new request: answers start over.
    pub fn restart(&mut self) {
        self.state = State::Fresh;
    }
    pub fn set(&mut self, credentials: Credentials) {
        self.credentials = Some(credentials);
        self.state = State::Fresh;
    }
    /// The header to send before any challenge: Basic when that is the
    /// only scheme the credentials are for (WinHTTP sends it at once).
    pub fn preemptive(&self) -> Option<String> {
        let c = self.credentials.as_ref()?;
        (c.schemes == BASIC && self.state == State::Fresh).then(|| basic(c))
    }
    /// The header answering `challenges` (the WWW-Authenticate or
    /// Proxy-Authenticate values of a 401/407), or None: no credentials, no
    /// scheme both sides know, or an answer was already refused.
    pub fn answer(&mut self, challenges: &[&str], r: &Request) -> Option<String> {
        let c = self.credentials.clone()?;
        if let State::Ntlm { negotiate, spnego } = &self.state {
            let (negotiate, spnego) = (negotiate.clone(), *spnego);
            self.state = State::Answered;
            let prefix = if spnego { "Negotiate" } else { "NTLM" };
            let token = challenges.iter().find_map(|h| {
                let (name, token) = h.trim().split_once(' ')?;
                name.eq_ignore_ascii_case(prefix).then(|| token.trim())
            })?;
            let token = base64_decode(token)?;
            // Under Negotiate the server may answer in SPNEGO; the reply
            // follows its form.
            let wrapped = spnego && !token.starts_with(b"NTLMSSP\0");
            let challenge = spnego::response_token(&token)?;
            let (user, domain) = ntlm::split_user(&c.user);
            let client: [u8; 8] = r.random[16..].try_into().unwrap();
            let id = ntlm::Identity {
                user: &user,
                domain: &domain,
                password: &c.password,
                workstation: r.workstation,
            };
            let message = ntlm::authenticate(&negotiate, &challenge, &id, client, r.now)?;
            let message = if wrapped {
                spnego::response(None, Some(&message))
            } else {
                message
            };
            return Some(format!("{prefix} {}", crate::base64(&message)));
        }
        let stale = challenges.iter().any(|h| {
            scheme_of(h) == DIGEST
                && digest::params(h, "Digest").is_some_and(|p| {
                    digest::param(&p, "stale").is_some_and(|s| s.eq_ignore_ascii_case("true"))
                })
        });
        if self.state == State::Answered && !stale {
            return None;
        }
        for bit in [NEGOTIATE, NTLM, DIGEST, BASIC] {
            if c.schemes & bit == 0 {
                continue;
            }
            let Some(challenge) = challenges.iter().find(|h| scheme_of(h) == bit) else {
                continue;
            };
            match bit {
                NEGOTIATE | NTLM => {
                    // Without Kerberos, Windows sends the bare NTLM token
                    // under Negotiate too (observed: WinHTTP on Windows 11).
                    let negotiate = ntlm::negotiate();
                    let spnego = bit == NEGOTIATE;
                    let token = crate::base64(&negotiate);
                    self.state = State::Ntlm { negotiate, spnego };
                    let name = if spnego { "Negotiate" } else { "NTLM" };
                    return Some(format!("{name} {token}"));
                }
                DIGEST => {
                    let answer = digest::Answer {
                        user: &c.user,
                        password: &c.password,
                        method: r.method,
                        uri: r.uri,
                        cnonce: r.random[..16].try_into().unwrap(),
                        count: 1,
                    };
                    let header = digest::authorization(challenge, &answer)?;
                    self.state = State::Answered;
                    return Some(header);
                }
                _ => {
                    self.state = State::Answered;
                    return Some(basic(&c));
                }
            }
        }
        None
    }
}
fn basic(c: &Credentials) -> String {
    format!(
        "Basic {}",
        crate::base64(format!("{}:{}", c.user, c.password).as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request<'static> {
        Request {
            method: "GET",
            uri: "/ntlm",
            random: [3; 24],
            now: 0x01d9_0000_0000_0000,
            workstation: "DOORS-PC",
        }
    }
    #[test]
    fn base64_round_trips() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", &[0xff, 0, 0x80][..]] {
            assert_eq!(base64_decode(&crate::base64(data)).unwrap(), data);
        }
        assert_eq!(base64_decode("!!"), None);
    }
    #[test]
    fn ntlm_takes_two_legs_and_verifies_on_the_server() {
        let mut c = Client::default();
        c.set(Credentials {
            user: "DOORS\\probe".into(),
            password: "s3cret".into(),
            schemes: NTLM,
        });
        let first = c
            .answer(&["NTLM", "Basic realm=\"x\""], &request())
            .unwrap();
        assert!(first.starts_with("NTLM TlRMTVNTUAAB"));
        let server = [4; 8];
        let challenge = ntlm::server_challenge(server, "DOORS", 1, ntlm::FLAGS);
        let header = format!("NTLM {}", crate::base64(&challenge));
        let second = c.answer(&[&header], &request()).unwrap();
        let token = base64_decode(second.strip_prefix("NTLM ").unwrap()).unwrap();
        assert_eq!(
            ntlm::server_verify(&token, server, "s3cret").as_deref(),
            Some("probe")
        );
        // A refusal after that is final.
        assert_eq!(c.answer(&["NTLM"], &request()), None);
    }
    #[test]
    fn negotiate_sends_bare_ntlm_and_follows_a_spnego_server() {
        let mut c = Client::default();
        c.set(Credentials {
            user: "probe".into(),
            password: "s3cret".into(),
            schemes: NEGOTIATE | NTLM,
        });
        let first = c.answer(&["Negotiate", "NTLM"], &request()).unwrap();
        let token = base64_decode(first.strip_prefix("Negotiate ").unwrap()).unwrap();
        // Bare NTLM first, as Windows sends it.
        assert_eq!(token, ntlm::negotiate());
        let server = [6; 8];
        let reply = spnego::response(
            Some(1),
            Some(&ntlm::server_challenge(server, "DOORS", 0, ntlm::FLAGS)),
        );
        let header = format!("Negotiate {}", crate::base64(&reply));
        let second = c.answer(&[&header], &request()).unwrap();
        let token = base64_decode(second.strip_prefix("Negotiate ").unwrap()).unwrap();
        let inner = spnego::init_token(&token).unwrap();
        assert_eq!(
            ntlm::server_verify(&inner, server, "s3cret").as_deref(),
            Some("probe")
        );
    }
    #[test]
    fn digest_is_chosen_over_basic_and_basic_is_preemptive_only_alone() {
        let mut c = Client::default();
        c.set(Credentials {
            user: "probe".into(),
            password: "s3cret".into(),
            schemes: BASIC | DIGEST,
        });
        assert_eq!(c.preemptive(), None);
        let header = c
            .answer(
                &[
                    "Basic realm=\"d\"",
                    "Digest realm=\"d\", nonce=\"n1\", qop=\"auth\"",
                ],
                &request(),
            )
            .unwrap();
        assert!(digest::verify(&header, "GET", "s3cret", "n1"));
        let mut b = Client::default();
        b.set(Credentials {
            user: "u".into(),
            password: "p".into(),
            schemes: BASIC,
        });
        assert_eq!(b.preemptive().as_deref(), Some("Basic dTpw"));
    }
}
