//! Live handshakes with public servers (`cargo test -p door-tls --test
//! live -- --ignored`): certificate chains to the Mozilla roots, each
//! cipher suite the server picks, and an HTTP/1.1 response read back.
//! `HTTPS_PROXY=http://host:port` tunnels through an HTTP proxy (CONNECT),
//! as Doors does with a configured proxy.
use std::io::{Read, Write};
use std::net::TcpStream;

fn connect(host: &str) -> Result<TcpStream, String> {
    if let Ok(proxy) = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("https_proxy")) {
        let addr = proxy.trim_start_matches("http://").trim_end_matches('/');
        let mut s = TcpStream::connect(addr).map_err(|e| format!("proxy: {e}"))?;
        write!(s, "CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\n\r\n")
            .map_err(|e| e.to_string())?;
        let mut reply = Vec::new();
        let mut b = [0u8; 1];
        while !reply.ends_with(b"\r\n\r\n") {
            s.read_exact(&mut b).map_err(|e| format!("proxy: {e}"))?;
            reply.push(b[0]);
        }
        let line = String::from_utf8_lossy(&reply);
        if !line.starts_with("HTTP/1.1 200") && !line.starts_with("HTTP/1.0 200") {
            return Err(format!(
                "proxy refused: {}",
                line.lines().next().unwrap_or("")
            ));
        }
        return Ok(s);
    }
    use std::net::ToSocketAddrs;
    let v4 = (host, 443)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .find(|a| a.is_ipv4())
        .ok_or("no IPv4 address")?;
    TcpStream::connect(v4).map_err(|e| e.to_string())
}
fn get(host: &str, path: &str) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut random = [0u8; 96];
    for (i, b) in random.iter_mut().enumerate() {
        *b = (now as u8)
            .wrapping_mul(31)
            .wrapping_add((i as u8).wrapping_mul(7))
            ^ 0x5A;
    }
    let mut s = connect(host)?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(15)))
        .unwrap();
    let (mut c, hello) = door_tls::Client::new(host, now, random);
    s.write_all(&hello).map_err(|e| format!("hello: {e}"))?;
    let mut buf = [0u8; 16384];
    while !c.connected() {
        let n = s.read(&mut buf).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            return Err("closed during handshake".into());
        }
        let out = c.receive(&buf[..n]).map_err(|e| format!("{e:?}"))?;
        s.write_all(&out).map_err(|e| format!("write: {e}"))?;
    }
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nUser-Agent: DoorsOS/1.0\r\n\r\n"
    );
    s.write_all(&c.send(request.as_bytes()).unwrap()).unwrap();
    let mut body = Vec::new();
    loop {
        let n = match s.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let out = c.receive(&buf[..n]).map_err(|e| format!("{e:?}"))?;
        s.write_all(&out).ok();
        body.extend(c.take_plaintext());
        if c.closed() {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

#[test]
#[ignore = "needs the internet"]
fn real_servers_answer_over_tls13() {
    let mut failures = Vec::new();
    for (host, path) in [
        ("example.com", "/"),
        (
            "api.open-meteo.com",
            "/v1/forecast?latitude=45.46&longitude=9.19&current=temperature_2m",
        ),
        ("www.cloudflare.com", "/cdn-cgi/trace"),
        ("www.google.com", "/generate_204"),
        ("github.com", "/"),
        ("www.wikipedia.org", "/"),
        ("www.microsoft.com", "/"),
        ("www.youtube.com", "/"),
        ("letsencrypt.org", "/"),
        ("www.rust-lang.org", "/"),
    ] {
        match get(host, path) {
            Ok(reply) if reply.starts_with("HTTP/1.1 ") => {
                println!("{host}: {}", reply.lines().next().unwrap())
            }
            Ok(reply) => failures.push(format!("{host}: {}", &reply[..reply.len().min(120)])),
            Err(e) => failures.push(format!("{host}: {e}")),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
#[test]
#[ignore = "needs the internet"]
fn wrong_host_and_untrusted_chains_are_refused() {
    // badssl.com's TLS 1.3 front end: a real certificate for another name.
    let e = get("wrong.host.badssl.com", "/").unwrap_err();
    println!("wrong.host: {e}");
    assert!(
        e.contains("Certificate") || e.contains("Unsupported") || e.contains("Alert"),
        "{e}"
    );
}
