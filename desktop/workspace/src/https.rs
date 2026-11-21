//! A non-blocking HTTPS GET over door.network and door-tls (P15), polled
//! from an app's tick: resolve, connect, TLS 1.3 handshake (certificates
//! checked against the Mozilla roots), one HTTP/1.1 request, and the
//! response read to its end (Content-Length, chunked, or close). Plain
//! `http://` is refused: apps only fetch over TLS. With a proxy server set
//! in Settings (`proxy`), the connection goes to the proxy and tunnels with
//! HTTP CONNECT, as WinINet does; TLS still runs end to end with the site.
extern crate alloc;
use alloc::{string::String, vec::Vec};
use door_service_client::Network;

/// Largest response body kept.
pub const MAX_BODY: usize = 4 * 1024 * 1024;
/// A fetch gives up after this long (ms).
pub const TIMEOUT_MS: u64 = 20_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
pub enum Poll {
    Pending,
    Done(Response),
    Failed(&'static str),
}
enum Stage {
    Resolving(u32),
    Connecting,
    /// Waiting for the proxy's answer to CONNECT.
    Tunnel,
    Handshake,
    Reading,
}
pub struct Fetch {
    net: Network,
    host: String,
    path: String,
    stage: Stage,
    socket: Option<u32>,
    tls: Option<door_tls::Client>,
    hello: Vec<u8>,
    pending: Vec<u8>,
    response: Vec<u8>,
    deadline: u64,
    /// The proxy this fetch tunnels through, if any.
    proxy: Option<crate::proxy::Proxy>,
    tunnel: Vec<u8>,
}
/// `https://host[:443]/path?query` split, or None.
pub fn split_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = host.strip_suffix(":443").unwrap_or(host);
    // Spaces and control characters would let the path inject header lines.
    let clean = |s: &str| s.bytes().all(|b| b > b' ' && b != 0x7F);
    (!host.is_empty() && !host.contains(':') && !host.contains('@') && clean(host) && clean(path))
        .then_some((host, path))
}
impl Fetch {
    pub fn start(registry: u64, url: &str, now: u64) -> Result<Self, &'static str> {
        let (host, path) = split_url(url).ok_or("Only https:// addresses can be fetched.")?;
        let mut net =
            Network::connect(registry).map_err(|_| "The network service isn't available.")?;
        let status = net
            .status()
            .map_err(|_| "The network service isn't available.")?;
        if !(status.present && status.link && status.enabled && status.address != [0; 4]) {
            return Err("You're offline.");
        }
        // The user's proxy (HKCU Internet Settings), unless this host is in
        // its exceptions.
        let proxy = door_service_client::Storage::connect(registry)
            .ok()
            .and_then(|mut s| crate::proxy::read(&mut s))
            .filter(|p| !p.bypassed(host));
        let target = proxy.as_ref().map_or(host, |p| p.host.as_str());
        let (query, address) = net.resolve(target).map_err(|_| {
            if proxy.is_some() {
                "The proxy server couldn't be looked up."
            } else {
                "The address couldn't be looked up."
            }
        })?;
        let mut f = Self {
            net,
            host: host.into(),
            path: path.into(),
            stage: Stage::Resolving(query),
            socket: None,
            tls: None,
            hello: Vec::new(),
            pending: Vec::new(),
            response: Vec::new(),
            deadline: now + TIMEOUT_MS,
            proxy,
            tunnel: Vec::new(),
        };
        if let Some(a) = address {
            f.connect(a)?;
        }
        Ok(f)
    }
    fn connect(&mut self, address: [u8; 4]) -> Result<(), &'static str> {
        let id = self
            .net
            .socket(door_service_protocol::net::TCP)
            .map_err(|_| "No network sockets are free.")?;
        let port = self.proxy.as_ref().map_or(443, |p| p.port);
        self.net.connect_to(id, address, port).map_err(|_| {
            if self.proxy.is_some() {
                "The proxy server couldn't be reached."
            } else {
                "The server couldn't be reached."
            }
        })?;
        self.socket = Some(id);
        let mut random = [0u8; 96];
        // The syscall reads at most `entropy::READ_MAX` (64) bytes at a time.
        if random
            .chunks_mut(48)
            .any(|c| door_sys::entropy_read(c) != Ok(c.len()))
        {
            return Err("The system's random source isn't available.");
        }
        // FILETIME (100 ns since 1601) to Unix seconds, for certificate dates.
        let filetime = door_sys::wall_time().map_err(|_| "The system clock isn't set.")?;
        let now = (filetime / 10_000_000) as i64 - 11_644_473_600;
        let (tls, hello) = door_tls::Client::new(&self.host, now, random);
        self.tls = Some(tls);
        self.hello = hello;
        self.stage = Stage::Connecting;
        Ok(())
    }
    /// Send queued bytes as far as the socket takes them.
    fn flush(&mut self) -> Result<(), &'static str> {
        let id = self.socket.ok_or("not connected")?;
        while !self.pending.is_empty() {
            let n = self
                .net
                .send(id, &self.pending)
                .map_err(|_| "The connection was lost.")?;
            if n == 0 {
                break;
            }
            self.pending.drain(..n);
        }
        Ok(())
    }
    pub fn poll(&mut self, now: u64) -> Poll {
        match self.step(now) {
            Ok(Some(r)) => Poll::Done(r),
            Ok(None) if now > self.deadline => Poll::Failed("The server took too long to answer."),
            Ok(None) => Poll::Pending,
            Err(e) => Poll::Failed(e),
        }
    }
    fn step(&mut self, _now: u64) -> Result<Option<Response>, &'static str> {
        use door_service_protocol::net::SocketState;
        if let Stage::Resolving(q) = self.stage {
            match self.net.resolved(q) {
                Ok(Some(a)) => {
                    let first = *a.list[..a.count]
                        .first()
                        .ok_or("The address couldn't be looked up.")?;
                    self.connect(first)?;
                }
                Ok(None) => return Ok(None),
                Err(_) => return Err("The address couldn't be looked up."),
            }
        }
        let id = self.socket.ok_or("not connected")?;
        if let Stage::Connecting = self.stage {
            match self
                .net
                .state(id)
                .map_err(|_| "The connection was lost.")?
                .0
            {
                SocketState::Established => {
                    if self.proxy.is_some() {
                        let connect = alloc::format!(
                            "CONNECT {0}:443 HTTP/1.1\r\nHost: {0}:443\r\nUser-Agent: DoorsOS/26H2\r\nProxy-Connection: keep-alive\r\n\r\n",
                            self.host
                        );
                        self.pending = connect.into_bytes();
                        self.stage = Stage::Tunnel;
                    } else {
                        self.pending = core::mem::take(&mut self.hello);
                        self.stage = Stage::Handshake;
                    }
                }
                SocketState::Closed => return Err("The server refused the connection."),
                _ => return Ok(None),
            }
        }
        self.flush()?;
        let mut buf = [0u8; 4096];
        for _ in 0..64 {
            let n = match self.net.receive(id, &mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => {
                    // The connection ended without TLS close_notify: only a
                    // response whose length is known can be complete.
                    return match self.finish(false)? {
                        Some(r) => Ok(Some(r)),
                        None => Err("The connection was lost."),
                    };
                }
            };
            let mut data = &buf[..n];
            let mut rest = Vec::new();
            if let Stage::Tunnel = self.stage {
                self.tunnel.extend_from_slice(data);
                if self.tunnel.len() > 8192 {
                    return Err("The proxy server's answer was too long.");
                }
                let Some(end) = self.tunnel.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let line = self.tunnel.split(|b| *b == b'\r').next().unwrap_or(&[]);
                let ok = line.starts_with(b"HTTP/1.1 200") || line.starts_with(b"HTTP/1.0 200");
                if !ok {
                    return Err(if line.windows(3).any(|w| w == b"407") {
                        "The proxy server needs a sign-in Doors doesn't support yet."
                    } else {
                        "The proxy server refused the connection."
                    });
                }
                rest = self.tunnel.split_off(end + 4);
                self.tunnel.clear();
                self.pending.extend(core::mem::take(&mut self.hello));
                self.stage = Stage::Handshake;
                self.flush()?;
                if rest.is_empty() {
                    continue;
                }
                data = &[];
            }
            let tls = self.tls.as_mut().ok_or("not connected")?;
            let input = if rest.is_empty() { data } else { &rest[..] };
            let out = tls.receive(input).map_err(|e| match e {
                door_tls::Error::Certificate(_) => "The site's security certificate isn't valid.",
                door_tls::Error::Unsupported => "The site doesn't support TLS 1.3.",
                _ => "The secure connection failed.",
            })?;
            self.pending.extend_from_slice(&out);
            self.response.extend(tls.take_plaintext());
            if self.response.len() > MAX_BODY + 64 * 1024 {
                return Err("The response is too large.");
            }
            if let Stage::Handshake = self.stage
                && tls.connected()
            {
                let request = alloc::format!(
                    "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: DoorsOS/26H2\r\nAccept: application/json, */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
                    self.path,
                    self.host
                );
                let records = tls
                    .send(request.as_bytes())
                    .map_err(|_| "The secure connection failed.")?;
                self.pending.extend_from_slice(&records);
                self.stage = Stage::Reading;
            }
            if tls.closed() {
                return self.finish(true);
            }
        }
        self.flush()?;
        if let Stage::Reading = self.stage {
            return self.finish(false);
        }
        Ok(None)
    }
    /// A complete response, or None while more is due (`closed`: nothing
    /// more will come).
    fn finish(&mut self, closed: bool) -> Result<Option<Response>, &'static str> {
        match parse(&self.response, closed) {
            Some(r) => {
                if let Some(id) = self.socket.take() {
                    let _ = self.net.close(id);
                }
                Ok(Some(r))
            }
            None if closed => Err("The server's answer was incomplete."),
            None => Ok(None),
        }
    }
}
impl Drop for Fetch {
    fn drop(&mut self) {
        if let Some(id) = self.socket.take() {
            let _ = self.net.close(id);
        }
    }
}
/// Parse an HTTP/1.1 response once complete (`closed`: the stream ended).
pub fn parse(raw: &[u8], closed: bool) -> Option<Response> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = core::str::from_utf8(&raw[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next()?;
    let status: u16 = status_line.split(' ').nth(1)?.parse().ok()?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), String::from(v.trim())))
        .collect();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let body = &raw[end + 4..];
    let body = if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        let mut out = Vec::new();
        let mut at = 0;
        loop {
            let line_end = body.get(at..)?.windows(2).position(|w| w == b"\r\n")? + at;
            let size_text = core::str::from_utf8(&body[at..line_end]).ok()?;
            let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
            at = line_end + 2;
            if size == 0 {
                break;
            }
            if size > MAX_BODY {
                return None;
            }
            let end = at.checked_add(size)?;
            out.extend_from_slice(body.get(at..end)?);
            at = end.checked_add(2)?;
            if out.len() > MAX_BODY {
                return None;
            }
        }
        out
    } else if let Some(len) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if body.len() < len || len > MAX_BODY {
            return None;
        }
        body[..len].to_vec()
    } else if closed {
        body.to_vec()
    } else {
        return None;
    };
    Some(Response {
        status,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_responses_are_refused_without_panicking() {
        let huge = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\nx";
        assert_eq!(parse(huge, true), None);
        let past = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx";
        assert_eq!(parse(past, true), None);
        let ok = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n";
        assert_eq!(parse(ok, true).map(|r| r.body), Some(b"hi".to_vec()));
        assert_eq!(split_url("https://a.example/x\r\nEvil: 1"), None);
        assert_eq!(split_url("https://a.example/a b"), None);
    }
    #[test]
    fn urls_and_responses() {
        assert_eq!(
            split_url("https://api.open-meteo.com/v1/x?a=1"),
            Some(("api.open-meteo.com", "/v1/x?a=1"))
        );
        assert_eq!(split_url("https://example.com"), Some(("example.com", "/")));
        assert_eq!(split_url("http://example.com/"), None);
        assert_eq!(split_url("https://user@evil/"), None);
        let r = parse(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi", false).unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, b"hi".as_slice()));
        assert!(parse(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhi", false).is_none());
        let c = parse(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n", false).unwrap();
        assert_eq!(c.body, b"abcde");
        assert_eq!(
            parse(b"HTTP/1.1 404 Not Found\r\n\r\nx", true)
                .unwrap()
                .status,
            404
        );
    }
}
