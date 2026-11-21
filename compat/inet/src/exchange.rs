//! One HTTP request/response over a kept-alive connection: the engine
//! WinHTTP and WinINet share. It connects (through a proxy when one
//! applies), negotiates TLS, sends the head and body, reads the response
//! head, follows redirects, answers server (401) and proxy (407)
//! challenges with the credentials it holds (Basic, Digest, NTLM and
//! Negotiate, `door_http::auth`), and hands out the body. The DLLs map
//! `Event`s to their status callbacks and `Failure`s to their error codes.
use crate::guest::{Deadline, Stream, now_unix, random, resolve, tunnel};
use crate::{Failure, Tolerance};
use alloc::{string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use door_http::auth;
use door_http::{Headers, Parser, Response, Scheme, Url};

/// Who a challenge comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Server,
    Proxy,
}
impl Target {
    fn challenge_header(self) -> &'static str {
        match self {
            Target::Server => "WWW-Authenticate",
            Target::Proxy => "Proxy-Authenticate",
        }
    }
    fn answer_header(self) -> &'static str {
        match self {
            Target::Server => "Authorization",
            Target::Proxy => "Proxy-Authorization",
        }
    }
}

/// What one exchange runs under (from the session and request options).
#[derive(Clone, Debug)]
pub struct Settings {
    /// Resolve, connect, send, receive, in milliseconds (0: no limit).
    pub timeouts: [u32; 4],
    pub proxy: Option<(String, u16)>,
    pub agent: String,
    pub follow_redirects: bool,
    /// Follow an https -> http redirect.
    pub allow_downgrade: bool,
    pub max_redirects: u32,
    pub tolerance: Tolerance,
    pub keep_alive: bool,
    /// Ask for and undo gzip/deflate content coding (WinHTTP's
    /// WINHTTP_OPTION_DECOMPRESSION, WinINet's INTERNET_OPTION_HTTP_DECODING).
    pub decode: bool,
}

/// Progress the DLLs report through their status callbacks.
pub enum Event<'a> {
    Resolving(&'a str),
    Resolved(&'a str),
    Connecting(&'a str),
    Connected(&'a str),
    Sending,
    Sent(u32),
    Receiving,
    Received(u32),
    Redirect(&'a str),
    SecureFailure(u32),
    ClosingConnection,
    ConnectionClosed,
}

/// Why `receive` stopped without a usable response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Failed(Failure),
    /// A redirect target was invalid, or too many redirects.
    RedirectFailed,
}
impl From<Failure> for Stop {
    fn from(f: Failure) -> Self {
        Stop::Failed(f)
    }
}

/// Cookies a session keeps: what to send for a URL, and what to store.
pub trait Cookies {
    fn header(&mut self, url: &Url) -> Option<String>;
    fn store(&mut self, url: &Url, set_cookie: &str, now: i64);
}
/// No cookie handling (INTERNET_FLAG_NO_COOKIES, WINHTTP_DISABLE_COOKIES).
pub struct NoCookies;
impl Cookies for NoCookies {
    fn header(&mut self, _url: &Url) -> Option<String> {
        None
    }
    fn store(&mut self, _url: &Url, _set_cookie: &str, _now: i64) {}
}

pub struct Exchange {
    pub verb: String,
    pub url: Url,
    pub version: String,
    /// Headers the caller added.
    pub headers: Headers,
    /// The headers of the last request actually sent.
    pub sent: Headers,
    /// Authentication with the server and with the proxy.
    pub server: auth::Client,
    pub proxy: auth::Client,
    /// Answers to send with the next request (server, proxy).
    answers: [Option<String>; 2],
    /// Body length promised and bytes written.
    pub total: u64,
    pub written: u64,
    pub redirects: u32,
    stream: Option<Stream>,
    parser: Option<Parser>,
    pub cancel: Arc<AtomicBool>,
    /// The decoded body of a gzip/deflate response, and how much of it was
    /// read; `None` while the body is read as it arrives.
    decoded: Option<(Vec<u8>, usize)>,
    /// This response's content coding is undone (`Settings::decode`).
    decoding: bool,
}

impl Exchange {
    pub fn new(
        verb: String,
        url: Url,
        version: String,
        headers: Headers,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        Self {
            verb,
            url,
            version,
            headers,
            sent: Headers::default(),
            server: auth::Client::default(),
            proxy: auth::Client::default(),
            answers: [None, None],
            total: 0,
            written: 0,
            redirects: 0,
            stream: None,
            parser: None,
            cancel,
            decoded: None,
            decoding: false,
        }
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
    /// Credentials for `schemes` (auth bits; WinINet passes all of them).
    pub fn set_credentials(
        &mut self,
        target: Target,
        user: String,
        password: String,
        schemes: u32,
    ) {
        let credentials = auth::Credentials {
            user,
            password,
            schemes,
        };
        match target {
            Target::Server => self.server.set(credentials),
            Target::Proxy => self.proxy.set(credentials),
        }
    }
    pub fn has_credentials(&self, target: Target) -> bool {
        match target {
            Target::Server => self.server.credentials.is_some(),
            Target::Proxy => self.proxy.credentials.is_some(),
        }
    }
    /// What an authentication answer needs from this request.
    fn auth_request(
        &self,
        uri: &str,
        f: impl FnOnce(&auth::Request) -> Option<String>,
    ) -> Option<String> {
        let mut bytes = [0u8; 24];
        if !random(&mut bytes) {
            return None;
        }
        // FILETIME: 100 ns units since 1601.
        let now = (now_unix().max(0) as u64 + 11_644_473_600) * 10_000_000;
        f(&auth::Request {
            method: &self.verb,
            uri,
            random: bytes,
            now,
            workstation: "",
        })
    }
    /// The header to send for `target`: an answer prepared from the last
    /// challenge, one the application's new credentials make from it, or
    /// Basic sent at once.
    fn authorization(&mut self, target: Target, uri: &str) -> Option<String> {
        let slot = target as usize;
        if let Some(answer) = self.answers[slot].take() {
            return Some(answer);
        }
        let client = match target {
            Target::Server => &self.server,
            Target::Proxy => &self.proxy,
        };
        if let Some(basic) = client.preemptive() {
            return Some(basic);
        }
        if client.credentials.is_none() || client.last.is_empty() {
            return None;
        }
        let mut client = client.clone();
        let last: Vec<String> = client.last.clone();
        let answer = self.auth_request(uri, |r| {
            let refs: Vec<&str> = last.iter().map(String::as_str).collect();
            client.answer(&refs, r)
        });
        match target {
            Target::Server => self.server = client,
            Target::Proxy => self.proxy = client,
        }
        answer
    }
    pub fn response(&self) -> Option<&Response> {
        self.parser.as_ref().and_then(Parser::response)
    }
    pub fn connected_securely(&self) -> bool {
        self.stream.as_ref().is_some_and(Stream::secure)
    }
    pub fn certificate_flags(&self) -> u32 {
        self.stream.as_ref().map_or(0, |s| s.certificate_flags)
    }
    /// Drops the connection (reporting it when there was one).
    pub fn disconnect(&mut self, notify: &mut dyn FnMut(Event<'_>)) {
        if let Some(mut stream) = self.stream.take() {
            notify(Event::ClosingConnection);
            stream.shutdown();
            drop(stream);
            notify(Event::ConnectionClosed);
        }
    }
    /// Opens (or reuses) the connection and sends the head and `optional`.
    pub fn send(
        &mut self,
        s: &Settings,
        cookies: &mut dyn Cookies,
        optional: &[u8],
        notify: &mut dyn FnMut(Event<'_>),
    ) -> Result<(), Failure> {
        let cancel = self.cancel.clone();
        let is_cancelled = move || cancel.load(Ordering::Acquire);
        let reusable = s.keep_alive
            && self.stream.as_ref().is_some_and(Stream::alive)
            && self.parser.as_ref().is_some_and(Parser::reusable);
        if !reusable {
            self.stream = None;
            let (host, port) = s
                .proxy
                .clone()
                .unwrap_or((self.url.host.clone(), self.url.port));
            notify(Event::Resolving(&host));
            let address = resolve(&host, Deadline::after(s.timeouts[0]))?;
            let text = alloc::format!(
                "{}.{}.{}.{}",
                address[0],
                address[1],
                address[2],
                address[3]
            );
            notify(Event::Resolved(&text));
            notify(Event::Connecting(&text));
            let deadline = Deadline::after(s.timeouts[1]);
            let mut stream = Stream::connect(address, port, deadline, &is_cancelled)?;
            notify(Event::Connected(&text));
            if self.url.scheme == Scheme::Https {
                // The CONNECT carries Basic proxy credentials when set.
                let proxy_basic = self.proxy.preemptive().or_else(|| {
                    let c = self.proxy.credentials.as_ref()?;
                    (c.schemes & auth::BASIC != 0).then(|| {
                        alloc::format!(
                            "Basic {}",
                            door_http::base64(
                                alloc::format!("{}:{}", c.user, c.password).as_bytes()
                            )
                        )
                    })
                });
                if s.proxy.is_some()
                    && tunnel(
                        &mut stream,
                        &self.url.host,
                        self.url.port,
                        proxy_basic.as_deref(),
                        deadline,
                        &is_cancelled,
                    )? != 200
                {
                    return Err(Failure::CannotConnect);
                }
                if let Err(f) =
                    stream.start_tls(&self.url.host, s.tolerance, deadline, &is_cancelled)
                {
                    if let Failure::Secure(flags) = f {
                        notify(Event::SecureFailure(flags));
                    }
                    return Err(f);
                }
            }
            self.stream = Some(stream);
        }
        let mut headers = self.headers.clone();
        if headers.get("User-Agent").is_none() && !s.agent.is_empty() {
            headers.set("User-Agent", &s.agent);
        }
        let has_body = self.total > 0
            || !optional.is_empty()
            || self.verb.eq_ignore_ascii_case("POST")
            || self.verb.eq_ignore_ascii_case("PUT");
        if has_body && headers.get("Content-Length").is_none() {
            headers.set(
                "Content-Length",
                &alloc::format!("{}", self.total.max(optional.len() as u64)),
            );
        }
        if let Some(cookie) = cookies.header(&self.url) {
            headers.coalesce("Cookie", &cookie, "; ");
        }
        // Through a plain-HTTP proxy the request line carries the URL.
        let target = if s.proxy.is_some() && self.url.scheme == Scheme::Http {
            self.url.to_text()
        } else {
            self.url.path.clone()
        };
        for who in [Target::Server, Target::Proxy] {
            // A header the application set itself wins.
            if headers.get(who.answer_header()).is_some()
                || (who == Target::Proxy && (s.proxy.is_none() || self.url.scheme == Scheme::Https))
            {
                continue;
            }
            if let Some(value) = self.authorization(who, &target) {
                headers.set(who.answer_header(), &value);
            }
        }
        if s.decode && headers.get("Accept-Encoding").is_none() {
            headers.set("Accept-Encoding", "gzip, deflate");
        }
        if !s.keep_alive && headers.get("Connection").is_none() {
            headers.set("Connection", "close");
        }
        let head = door_http::request_head(
            &self.verb,
            &target,
            &self.version,
            &self.url.host_header(),
            &headers,
        );
        self.sent = headers;
        notify(Event::Sending);
        let deadline = Deadline::after(s.timeouts[2]);
        let stream = self.stream.as_mut().ok_or(Failure::ConnectionError)?;
        stream.write_all(&head, deadline, &is_cancelled)?;
        stream.write_all(optional, deadline, &is_cancelled)?;
        self.written = optional.len() as u64;
        self.parser = None;
        self.decoded = None;
        self.decoding = false;
        notify(Event::Sent((head.len() + optional.len()) as u32));
        Ok(())
    }
    /// More of the request body after `send` (WinHttpWriteData,
    /// InternetWriteFile).
    pub fn write(&mut self, data: &[u8], s: &Settings) -> Result<(), Failure> {
        let cancel = self.cancel.clone();
        let stream = self.stream.as_mut().ok_or(Failure::ConnectionError)?;
        stream.write_all(data, Deadline::after(s.timeouts[2]), &|| {
            cancel.load(Ordering::Acquire)
        })?;
        self.written += data.len() as u64;
        Ok(())
    }
    /// Reads bytes into the parser; false when the connection ended.
    fn pump(&mut self, deadline: Deadline) -> Result<bool, Failure> {
        let cancel = self.cancel.clone();
        let stream = self.stream.as_mut().ok_or(Failure::ConnectionError)?;
        let bytes = stream.read(deadline, &|| cancel.load(Ordering::Acquire))?;
        let parser = self.parser.as_mut().ok_or(Failure::ConnectionError)?;
        if bytes.is_empty() {
            parser.close().map_err(|_| Failure::ConnectionError)?;
            return Ok(false);
        }
        parser.feed(&bytes).map_err(|_| Failure::InvalidResponse)?;
        Ok(true)
    }
    /// Reads the response head; follows redirects and answers a Basic
    /// challenge once when credentials are set.
    pub fn receive(
        &mut self,
        s: &Settings,
        cookies: &mut dyn Cookies,
        notify: &mut dyn FnMut(Event<'_>),
    ) -> Result<(), Stop> {
        loop {
            notify(Event::Receiving);
            let deadline = Deadline::after(s.timeouts[3]);
            let head_request = self.verb.eq_ignore_ascii_case("HEAD");
            self.parser.get_or_insert_with(|| Parser::new(head_request));
            let mut total = 0u32;
            while self.response().is_none() {
                if self.cancelled() {
                    return Err(Stop::Failed(Failure::Cancelled));
                }
                let before = self.parser.as_ref().map_or(0, Parser::body_available);
                if !self.pump(deadline)? && self.response().is_none() {
                    return Err(Stop::Failed(Failure::InvalidResponse));
                }
                total = total.saturating_add(
                    (self.parser.as_ref().map_or(0, Parser::body_available) - before) as u32,
                );
            }
            notify(Event::Received(total));
            let response = self
                .response()
                .cloned()
                .ok_or(Stop::Failed(Failure::InvalidResponse))?;
            let now = now_unix();
            for (name, value) in &response.headers.0 {
                if name.eq_ignore_ascii_case("Set-Cookie") {
                    cookies.store(&self.url, value, now);
                }
            }
            if let (Some(location), Some(method), true) = (
                response.headers.get("Location"),
                door_http::redirect_method(response.status, &self.verb),
                s.follow_redirects,
            ) {
                let target =
                    door_http::resolve(&self.url, location).map_err(|_| Stop::RedirectFailed)?;
                let downgrade = self.url.scheme == Scheme::Https && target.scheme == Scheme::Http;
                if !downgrade || s.allow_downgrade {
                    if self.redirects >= s.max_redirects {
                        return Err(Stop::RedirectFailed);
                    }
                    notify(Event::Redirect(&target.to_text()));
                    self.drain();
                    let changed_origin = target.host != self.url.host
                        || target.port != self.url.port
                        || target.scheme != self.url.scheme;
                    self.redirects += 1;
                    if !method.eq_ignore_ascii_case(&self.verb) {
                        self.headers.remove("Content-Length");
                        self.headers.remove("Content-Type");
                        self.total = 0;
                    }
                    self.verb = method;
                    if changed_origin {
                        // Credentials belong to the origin that asked.
                        self.disconnect(notify);
                        self.server = auth::Client::default();
                        self.answers[Target::Server as usize] = None;
                    }
                    self.url = target;
                    self.send(s, cookies, &[], notify)?;
                    continue;
                }
            }
            if response.status == 401 || response.status == 407 {
                let who = if response.status == 401 {
                    Target::Server
                } else {
                    Target::Proxy
                };
                let challenges: Vec<String> = response
                    .headers
                    .0
                    .iter()
                    .filter(|(n, _)| n.eq_ignore_ascii_case(who.challenge_header()))
                    .map(|(_, v)| v.clone())
                    .collect();
                let target = if s.proxy.is_some() && self.url.scheme == Scheme::Http {
                    self.url.to_text()
                } else {
                    self.url.path.clone()
                };
                let mut client = match who {
                    Target::Server => self.server.clone(),
                    Target::Proxy => self.proxy.clone(),
                };
                client.last = challenges.clone();
                // An answer the application set itself is not replaced.
                let answer = if self.headers.get(who.answer_header()).is_some() {
                    None
                } else {
                    self.auth_request(&target, |r| {
                        let refs: Vec<&str> = challenges.iter().map(String::as_str).collect();
                        client.answer(&refs, r)
                    })
                };
                match who {
                    Target::Server => self.server = client,
                    Target::Proxy => self.proxy = client,
                }
                if let Some(answer) = answer {
                    self.answers[who as usize] = Some(answer);
                    self.drain();
                    self.send(s, cookies, &[], notify)?;
                    continue;
                }
            } else {
                // A final answer: the next request starts its own exchange.
                self.server.restart();
                self.proxy.restart();
            }
            self.decoding = s.decode
                && response
                    .headers
                    .get("Content-Encoding")
                    .and_then(Coding::parse)
                    .is_some();
            return Ok(());
        }
    }
    /// The whole body, decoded (gzip/deflate responses when decoding).
    fn decode_body(&mut self, timeout: u32) -> Result<(), Failure> {
        let coding = self
            .response()
            .and_then(|r| r.headers.get("Content-Encoding"))
            .and_then(Coding::parse)
            .ok_or(Failure::InvalidResponse)?;
        let deadline = Deadline::after(timeout);
        let mut raw = Vec::new();
        loop {
            let parser = self.parser.as_mut().ok_or(Failure::ConnectionError)?;
            raw.extend(parser.take_body(usize::MAX));
            if raw.len() > MAX_ENCODED {
                return Err(Failure::InvalidResponse);
            }
            if parser.finished() && parser.body_available() == 0 {
                break;
            }
            if self.cancelled() {
                return Err(Failure::Cancelled);
            }
            if !self.pump(deadline)? {
                let parser = self.parser.as_mut().ok_or(Failure::ConnectionError)?;
                raw.extend(parser.take_body(usize::MAX));
                break;
            }
        }
        let body = coding.decode(&raw).ok_or(Failure::InvalidResponse)?;
        self.decoded = Some((body, 0));
        Ok(())
    }
    /// Discards the rest of the body so the connection can carry the next
    /// request; a connection that cannot be reused is dropped.
    pub fn drain(&mut self) {
        let deadline = Deadline::after(5000);
        for _ in 0..4096 {
            match self.parser.as_mut() {
                Some(p) if !p.finished() => {
                    p.take_body(usize::MAX);
                }
                _ => break,
            }
            if !matches!(self.pump(deadline), Ok(true)) {
                break;
            }
        }
        if let Some(p) = self.parser.as_mut() {
            p.take_body(usize::MAX);
        }
        if !self.parser.as_ref().is_some_and(Parser::reusable) {
            self.stream = None;
        }
    }
    /// Waits until body bytes are buffered or the body ended; how many.
    pub fn available(&mut self, timeout: u32) -> Result<usize, Failure> {
        if self.decoding {
            if self.decoded.is_none() {
                self.decode_body(timeout)?;
            }
            return Ok(self.decoded.as_ref().map_or(0, |(b, at)| b.len() - at));
        }
        let deadline = Deadline::after(timeout);
        loop {
            let parser = self.parser.as_ref().ok_or(Failure::ConnectionError)?;
            if parser.body_available() > 0 || parser.finished() {
                return Ok(parser.body_available());
            }
            if self.cancelled() {
                return Err(Failure::Cancelled);
            }
            self.pump(deadline)?;
        }
    }
    /// Up to `max` body bytes; with `fill`, waits until `max` bytes or the
    /// end (WinINet's InternetReadFile), else returns what is available
    /// (WinHttpReadData). Empty at the end of the body.
    pub fn read(&mut self, max: usize, fill: bool, timeout: u32) -> Result<Vec<u8>, Failure> {
        let mut out = Vec::new();
        while out.len() < max {
            if self.available(timeout)? == 0 {
                break;
            }
            if let Some((body, at)) = self.decoded.as_mut() {
                let n = (max - out.len()).min(body.len() - *at);
                out.extend_from_slice(&body[*at..*at + n]);
                *at += n;
            } else {
                let parser = self.parser.as_mut().ok_or(Failure::ConnectionError)?;
                out.extend(parser.take_body(max - out.len()));
            }
            if !fill {
                break;
            }
        }
        Ok(out)
    }
    /// Answer from a stored response (WinINet's URL cache): its head and
    /// body, with no connection; the body's own length replaces any
    /// framing the stored head names.
    pub fn from_store(&mut self, head: &str, body: &[u8]) {
        self.stream = None;
        self.decoded = None;
        self.decoding = false;
        let mut text = String::new();
        for line in head.split("\r\n").filter(|l| !l.is_empty()) {
            let name = line.split(':').next().unwrap_or("").trim();
            if name.eq_ignore_ascii_case("Content-Length")
                || name.eq_ignore_ascii_case("Transfer-Encoding")
                || name.eq_ignore_ascii_case("Content-Encoding")
            {
                continue;
            }
            text.push_str(line);
            text.push_str("\r\n");
        }
        text.push_str(&alloc::format!("Content-Length: {}\r\n\r\n", body.len()));
        let mut parser = Parser::new(false);
        let fed = parser
            .feed(text.as_bytes())
            .and_then(|()| parser.feed(body));
        self.parser = fed.is_ok().then_some(parser);
    }
    /// The body has been read to its end.
    pub fn finished(&self) -> bool {
        if let Some((body, at)) = &self.decoded {
            return *at == body.len();
        }
        !self.decoding
            && self
                .parser
                .as_ref()
                .is_some_and(|p| p.finished() && p.body_available() == 0)
    }
}

/// A body larger than this (encoded or decoded) is refused rather than
/// held in memory.
const MAX_ENCODED: usize = 32 * 1024 * 1024;
const MAX_DECODED: usize = 128 * 1024 * 1024;
/// The content codings WinHTTP and WinINet undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coding {
    Gzip,
    Deflate,
}
impl Coding {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("gzip") || value.eq_ignore_ascii_case("x-gzip") {
            Some(Self::Gzip)
        } else if value.eq_ignore_ascii_case("deflate") {
            Some(Self::Deflate)
        } else {
            None
        }
    }
    /// The decoded bytes; "deflate" is zlib-wrapped (RFC 9110), with a raw
    /// stream accepted as servers send that too.
    pub fn decode(self, data: &[u8]) -> Option<Vec<u8>> {
        use zune_inflate::{DeflateDecoder, DeflateOptions};
        let options = || DeflateOptions::default().set_limit(MAX_DECODED);
        let mut d = DeflateDecoder::new_with_options(data, options());
        match self {
            Self::Gzip => d.decode_gzip().ok(),
            Self::Deflate => d.decode_zlib().ok().or_else(|| {
                DeflateDecoder::new_with_options(data, options())
                    .decode_deflate()
                    .ok()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gzip_and_deflate_bodies_decode_and_bad_ones_do_not() {
        let gzip = [
            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0xcb, 0x48, 0xcd, 0xc9,
            0xc9, 0x57, 0x48, 0xc9, 0xcf, 0x2f, 0x2a, 0x06, 0x00, 0xec, 0x76, 0x66, 0x69, 0x0b,
            0x00, 0x00, 0x00,
        ];
        assert_eq!(Coding::parse(" GZIP "), Some(Coding::Gzip));
        assert_eq!(Coding::parse("x-gzip"), Some(Coding::Gzip));
        assert_eq!(Coding::parse("deflate"), Some(Coding::Deflate));
        assert_eq!(Coding::parse("br"), None);
        assert_eq!(Coding::Gzip.decode(&gzip).unwrap(), b"hello doors");
        // A raw deflate stream sent as "deflate" is accepted too.
        assert_eq!(
            Coding::Deflate.decode(&gzip[10..23]).unwrap(),
            b"hello doors"
        );
        assert!(Coding::Gzip.decode(&gzip[..12]).is_none());
    }
}
