//! The CONNECT-A test web server: plain HTTP/1.1 on this host's loopback,
//! reached by the guest at 10.0.2.2. Fixed routes exercise what WinHTTP
//! and WinINet must handle: Content-Length and chunked bodies, a redirect,
//! request echo, cookies, Basic, Digest, NTLM and Negotiate authentication,
//! keep-alive and 404.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub const HELLO: &str = "hello winhttp";
/// The proxy auto-config script /proxy.pac serves.
pub const PAC: &str = "function FindProxyForURL(url, host) {\n\
    if (isPlainHostName(host) || dnsDomainIs(host, \".direct.test\"))\n\
        return \"DIRECT\";\n\
    if (shExpMatch(url, \"*/via-proxy/*\"))\n\
        return \"PROXY proxy.doors.test:3128; DIRECT\";\n\
    return \"PROXY fallback.doors.test:8080\";\n\
}\n";
/// Basic credentials /private accepts ("doors:secret").
pub const CREDENTIALS: &str = "ZG9vcnM6c2VjcmV0";

/// The password /digest, /ntlm and /negotiate accept (user "doors"; NTLM
/// takes any domain).
pub const PASSWORD: &str = "secret";
const DIGEST_NONCE: &str = "dcd98b7102dd2f0e8b11d0f600bfb0c093";
/// The NTLM server challenge.
const NTLM_CHALLENGE: [u8; 8] = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
#[derive(Debug)]
enum Leg {
    /// A CHALLENGE_MESSAGE to send (base64, wrapped as the scheme wants).
    Challenge(String),
    Accepted(String),
    Refused,
}
/// One NTLM leg of /ntlm or /negotiate.
fn ntlm_leg(request: &Request, scheme: &str) -> Leg {
    use door_http::auth::{base64_decode, ntlm, spnego};
    let Some(token) = request
        .header("Authorization")
        .and_then(|a| a.split_once(' '))
        .filter(|(s, _)| s.eq_ignore_ascii_case(scheme))
        .and_then(|(_, t)| base64_decode(t.trim()))
    else {
        return Leg::Refused;
    };
    // Negotiate carries bare NTLM (Windows without Kerberos) or SPNEGO;
    // the answer takes the client's form.
    let negotiate = scheme == "Negotiate" && !token.starts_with(b"NTLMSSP\0");
    let Some(message) = (if negotiate {
        spnego::init_token(&token)
    } else {
        Some(token)
    }) else {
        return Leg::Refused;
    };
    // An optimistic token of another mechanism (Kerberos, NegoEx): SPNEGO
    // names NTLM and asks for its first token (RFC 4178 §3.2).
    if negotiate && !message.starts_with(b"NTLMSSP\0") {
        return Leg::Challenge(door_http::base64(&spnego::response(Some(1), None)));
    }
    match message.get(8) {
        Some(1) => {
            // FILETIME of the moment, for MsvAvTimestamp.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let requested = ntlm::negotiate_flags(&message).unwrap_or(ntlm::FLAGS);
            let challenge = ntlm::server_challenge(
                NTLM_CHALLENGE,
                "DOORS",
                (now + 11_644_473_600) * 10_000_000,
                requested,
            );
            let wrapped = if negotiate {
                spnego::response(Some(1), Some(&challenge))
            } else {
                challenge
            };
            Leg::Challenge(door_http::base64(&wrapped))
        }
        Some(3) => match ntlm::server_verify(&message, NTLM_CHALLENGE, PASSWORD) {
            Some(user) => Leg::Accepted(user),
            None => Leg::Refused,
        },
        _ => Leg::Refused,
    }
}

pub struct Server {
    pub port: u16,
    stop: Arc<AtomicBool>,
    pub log: Arc<Mutex<Vec<String>>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}
/// Reads one request (head and Content-Length body); None at the end of
/// the connection.
fn read_request(stream: &mut TcpStream, pending: &mut Vec<u8>) -> Option<Request> {
    let mut buffer = [0u8; 4096];
    let end = loop {
        if let Some(i) = pending.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let n = stream.read(&mut buffer).ok().filter(|n| *n > 0)?;
        pending.extend_from_slice(&buffer[..n]);
    };
    let head = String::from_utf8_lossy(&pending[..end]).into_owned();
    pending.drain(..end + 4);
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let path = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let length = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("Content-Length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    while pending.len() < length {
        let n = stream.read(&mut buffer).ok().filter(|n| *n > 0)?;
        pending.extend_from_slice(&buffer[..n]);
    }
    let body = pending.drain(..length).collect();
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}
fn respond(
    stream: &mut TcpStream,
    status: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nServer: Doors-Test\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (n, v) in headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)
}
fn serve_connection(mut stream: TcpStream, log: Arc<Mutex<Vec<String>>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut pending = Vec::new();
    // A TLS ClientHello on this plain port gets what web servers answer:
    // 400 and a close.
    let mut first = [0u8; 1];
    if stream.peek(&mut first).is_ok_and(|n| n == 1) && first[0] == 0x16 {
        if let Ok(mut log) = log.lock() {
            log.push(String::from("TLS handshake on the plain port: 400"));
        }
        // Read the ClientHello, answer, and close only after the peer did,
        // so no unread data turns the close into a reset.
        let mut hello = [0u8; 4096];
        let _ = stream.read(&mut hello);
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nServer: Doors-Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        while matches!(stream.read(&mut hello), Ok(n) if n > 0) {}
        return;
    }
    while let Some(request) = read_request(&mut stream, &mut pending) {
        if let Ok(mut log) = log.lock() {
            log.push(format!(
                "{} {} ({} body bytes)",
                request.method,
                request.path,
                request.body.len()
            ));
        }
        let result = match request.path.as_str() {
            "/hello" => respond(
                &mut stream,
                "200 OK",
                &[("Content-Type", "text/plain".into())],
                HELLO.as_bytes(),
            ),
            // A response WinINet may cache for an hour.
            "/cached" => respond(
                &mut stream,
                "200 OK",
                &[
                    ("Content-Type", "text/plain".into()),
                    ("Cache-Control", "public, max-age=3600".into()),
                    ("Last-Modified", "Sun, 08 Nov 2026 12:00:00 GMT".into()),
                ],
                b"cached by wininet",
            ),
            // Proxy auto-config: a proxy for /via-proxy/ URLs, DIRECT for
            // plain host names and *.direct.test.
            "/proxy.pac" => respond(
                &mut stream,
                "200 OK",
                &[("Content-Type", "application/x-ns-proxy-autoconfig".into())],
                PAC.as_bytes(),
            ),
            "/bad.pac" => respond(
                &mut stream,
                "200 OK",
                &[("Content-Type", "application/x-ns-proxy-autoconfig".into())],
                b"function FindProxyForURL(url, host) { return ]; }",
            ),
            "/chunked" => {
                let head = "HTTP/1.1 200 OK\r\nServer: Doors-Test\r\nTransfer-Encoding: chunked\r\nContent-Type: text/plain\r\n\r\n";
                let mut body = String::new();
                for part in ["chunked ", "bodies ", "work"] {
                    body.push_str(&format!("{:x}\r\n{part}\r\n", part.len()));
                }
                body.push_str("0\r\n\r\n");
                stream
                    .write_all(head.as_bytes())
                    .and_then(|()| stream.write_all(body.as_bytes()))
            }
            // gzip for a client that asks for it (Accept-Encoding).
            "/gzip" => {
                if request
                    .header("Accept-Encoding")
                    .is_some_and(|a| a.contains("gzip"))
                {
                    respond(
                        &mut stream,
                        "200 OK",
                        &[("Content-Encoding", "gzip".into())],
                        &GZIPPED,
                    )
                } else {
                    respond(&mut stream, "200 OK", &[], DECOMPRESSED.as_bytes())
                }
            }
            "/redirect" => respond(
                &mut stream,
                "302 Found",
                &[("Location", "/hello".into())],
                b"",
            ),
            "/echo" => {
                let reflected = request.header("X-Doors").unwrap_or("").to_owned();
                respond(
                    &mut stream,
                    "200 OK",
                    &[
                        ("X-Echo-Doors", reflected),
                        ("X-Echo-Method", request.method.clone()),
                    ],
                    &request.body,
                )
            }
            "/set-cookie" => respond(
                &mut stream,
                "200 OK",
                &[("Set-Cookie", "session=abc; Path=/".into())],
                b"set",
            ),
            "/cookie" => respond(
                &mut stream,
                "200 OK",
                &[],
                request.header("Cookie").unwrap_or("").as_bytes(),
            ),
            "/private" => {
                if request.header("Authorization") == Some(&format!("Basic {CREDENTIALS}")) {
                    respond(&mut stream, "200 OK", &[], b"welcome")
                } else {
                    respond(
                        &mut stream,
                        "401 Unauthorized",
                        &[("WWW-Authenticate", "Basic realm=\"doors\"".into())],
                        b"denied",
                    )
                }
            }
            // Digest (RFC 7616, MD5, qop=auth), offered beside Basic.
            "/digest" => {
                let ok = request.header("Authorization").is_some_and(|a| {
                    door_http::auth::digest::verify(a, &request.method, PASSWORD, DIGEST_NONCE)
                });
                if ok {
                    respond(&mut stream, "200 OK", &[], b"digest ok")
                } else {
                    respond(
                        &mut stream,
                        "401 Unauthorized",
                        &[
                            (
                                "WWW-Authenticate",
                                format!(
                                    "Digest realm=\"doors\", qop=\"auth\", nonce=\"{DIGEST_NONCE}\", opaque=\"5ccc069c\""
                                ),
                            ),
                            ("WWW-Authenticate", "Basic realm=\"doors\"".into()),
                        ],
                        b"denied",
                    )
                }
            }
            // NTLM, bare or inside Negotiate (SPNEGO): two legs on this
            // connection, the second checked against PASSWORD.
            "/ntlm" | "/negotiate" => {
                let scheme = if request.path == "/ntlm" {
                    "NTLM"
                } else {
                    "Negotiate"
                };
                let result = ntlm_leg(&request, scheme);
                if let Ok(mut log) = log.lock() {
                    log.push(format!("{scheme} leg: {result:?}"));
                }
                match result {
                    Leg::Accepted(user) => respond(
                        &mut stream,
                        "200 OK",
                        &[],
                        format!("{} ok {user}", scheme.to_lowercase()).as_bytes(),
                    ),
                    Leg::Challenge(token) => respond(
                        &mut stream,
                        "401 Unauthorized",
                        &[("WWW-Authenticate", format!("{scheme} {token}"))],
                        b"",
                    ),
                    Leg::Refused => respond(
                        &mut stream,
                        "401 Unauthorized",
                        &[("WWW-Authenticate", scheme.into())],
                        b"denied",
                    ),
                }
            }
            _ => respond(
                &mut stream,
                "404 Not Found",
                &[("Content-Type", "text/plain".into())],
                b"missing",
            ),
        };
        if result.is_err()
            || request
                .header("Connection")
                .is_some_and(|c| c.eq_ignore_ascii_case("close"))
        {
            break;
        }
    }
}
/// Serves until dropped.
pub fn serve() -> Result<Server, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let log = Arc::new(Mutex::new(Vec::new()));
    let (s, l) = (stop.clone(), log.clone());
    thread::spawn(move || {
        while !s.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let log = l.clone();
                    thread::spawn(move || serve_connection(stream, log));
                }
                Err(_) => thread::sleep(Duration::from_millis(10)),
            }
        }
    });
    Ok(Server { port, stop, log })
}

/// "decompressed by the client", gzip-encoded (as .NET's GZipStream wrote it).
const DECOMPRESSED: &str = "decompressed by the client";
const GZIPPED: [u8; 46] = [
    0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x4b, 0x49, 0x4d, 0xce, 0xcf, 0x2d,
    0x28, 0x4a, 0x2d, 0x2e, 0x4e, 0x4d, 0x51, 0x48, 0xaa, 0x54, 0x28, 0xc9, 0x48, 0x55, 0x48, 0xce,
    0xc9, 0x4c, 0xcd, 0x2b, 0x01, 0x00, 0x02, 0x08, 0xa2, 0x98, 0x1a, 0x00, 0x00, 0x00,
];

#[cfg(test)]
mod tests {
    use super::*;
    fn exchange(port: u16, request: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(request.as_bytes()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }
    #[test]
    fn routes_answer() {
        let server = serve().unwrap();
        let hello = exchange(
            server.port,
            "GET /hello HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert!(hello.starts_with("HTTP/1.1 200 OK\r\n") && hello.ends_with(HELLO));
        let chunked = exchange(
            server.port,
            "GET /chunked HTTP/1.1\r\nConnection: close\r\n\r\n",
        );
        assert!(chunked.contains("8\r\nchunked \r\n") && chunked.ends_with("0\r\n\r\n"));
        let echo = exchange(
            server.port,
            "POST /echo HTTP/1.1\r\nX-Doors: 7\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc",
        );
        assert!(echo.contains("X-Echo-Doors: 7") && echo.ends_with("abc"));
        let private = exchange(
            server.port,
            &format!(
                "GET /private HTTP/1.1\r\nAuthorization: Basic {CREDENTIALS}\r\nConnection: close\r\n\r\n"
            ),
        );
        assert!(private.ends_with("welcome"));
        assert!(
            exchange(
                server.port,
                "GET /nope HTTP/1.1\r\nConnection: close\r\n\r\n"
            )
            .starts_with("HTTP/1.1 404 Not Found")
        );
    }
}
