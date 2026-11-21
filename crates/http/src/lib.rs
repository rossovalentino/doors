#![no_std]
//! A sans-IO HTTP/1.1 client core (RFC 9110/9112) shared by Doors' WinHTTP
//! and WinINet: URL cracking with Windows' component rules, request heads,
//! an incremental response parser (status line, headers, Content-Length,
//! chunked with trailers, read-until-close, 1xx interim responses) and
//! redirect resolution (RFC 3986 reference resolution).
extern crate alloc;
pub mod auth;
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}
impl Scheme {
    pub fn default_port(self) -> u16 {
        match self {
            Scheme::Http => 80,
            Scheme::Https => 443,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub scheme: Scheme,
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: u16,
    /// Path and query, always starting with `/`.
    pub path: String,
    /// The `#fragment`, without `#` (never sent).
    pub fragment: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlError {
    /// Not a URL (no scheme, empty host, bad port).
    Invalid,
    /// A scheme other than http/https.
    UnrecognizedScheme,
}

/// Parses an absolute http/https URL. The scheme and host compare
/// case-insensitively; the host keeps its spelling.
pub fn parse_url(text: &str) -> Result<Url, UrlError> {
    let text = text.trim();
    let (scheme, rest) = text.split_once("://").ok_or(UrlError::Invalid)?;
    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        s if !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b)) =>
        {
            return Err(UrlError::UnrecognizedScheme);
        }
        _ => return Err(UrlError::Invalid),
    };
    let (rest, fragment) = match rest.split_once('#') {
        Some((r, f)) => (r, f.to_string()),
        None => (rest, String::new()),
    };
    let split = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(split);
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    let (user, password) = match userinfo {
        Some(u) => match u.split_once(':') {
            Some((n, p)) => (n.to_string(), p.to_string()),
            None => (u.to_string(), String::new()),
        },
        None => (String::new(), String::new()),
    };
    let (host, port) = if let Some(stripped) = hostport.strip_prefix('[') {
        // An IPv6 literal keeps its brackets out of the host.
        let (inside, after) = stripped.split_once(']').ok_or(UrlError::Invalid)?;
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p),
            None if after.is_empty() => None,
            None => return Err(UrlError::Invalid),
        };
        (inside, port)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        }
    };
    if host.is_empty() {
        return Err(UrlError::Invalid);
    }
    let port = match port {
        Some("") | None => scheme.default_port(),
        Some(p) => p
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or(UrlError::Invalid)?,
    };
    let path = if path.is_empty() {
        String::from("/")
    } else if path.starts_with('?') {
        alloc::format!("/{path}")
    } else {
        path.to_string()
    };
    Ok(Url {
        scheme,
        user,
        password,
        host: host.to_string(),
        port,
        path,
        fragment,
    })
}

impl Url {
    /// `host` or `host:port` as the Host header carries it (the port only
    /// when it is not the scheme's default).
    pub fn host_header(&self) -> String {
        let host = if self.host.contains(':') {
            alloc::format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == self.scheme.default_port() {
            host
        } else {
            alloc::format!("{host}:{}", self.port)
        }
    }
    pub fn to_text(&self) -> String {
        let mut s = alloc::format!("{}://", self.scheme.name());
        if !self.user.is_empty() {
            s.push_str(&self.user);
            if !self.password.is_empty() {
                s.push(':');
                s.push_str(&self.password);
            }
            s.push('@');
        }
        s.push_str(&self.host_header());
        s.push_str(&self.path);
        if !self.fragment.is_empty() {
            s.push('#');
            s.push_str(&self.fragment);
        }
        s
    }
}

/// RFC 3986 section 5.2: resolves a Location value against `base`.
pub fn resolve(base: &Url, reference: &str) -> Result<Url, UrlError> {
    let reference = reference.trim();
    if reference.contains("://") {
        return parse_url(reference);
    }
    if let Some(rest) = reference.strip_prefix("//") {
        return parse_url(&alloc::format!("{}://{rest}", base.scheme.name()));
    }
    let mut url = base.clone();
    url.fragment = String::new();
    let (reference, fragment) = match reference.split_once('#') {
        Some((r, f)) => (r, f.to_string()),
        None => (reference, String::new()),
    };
    url.fragment = fragment;
    if reference.is_empty() {
        return Ok(url);
    }
    let (base_path, _) = base.path.split_once('?').unwrap_or((&base.path, ""));
    let merged = if let Some(query) = reference.strip_prefix('?') {
        alloc::format!("{base_path}?{query}")
    } else if reference.starts_with('/') {
        reference.to_string()
    } else {
        let directory = &base_path[..base_path.rfind('/').map_or(0, |i| i + 1)];
        alloc::format!("{directory}{reference}")
    };
    url.path = remove_dot_segments(&merged);
    Ok(url)
}
fn remove_dot_segments(path: &str) -> String {
    let (path, query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    let mut out: Vec<&str> = Vec::new();
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len() - 1;
    for (i, segment) in segments.iter().enumerate() {
        match *segment {
            "." => {
                if i == last {
                    out.push("");
                }
            }
            ".." => {
                if out.len() > 1 {
                    out.pop();
                }
                if i == last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    let mut joined = out.join("/");
    if !joined.starts_with('/') {
        joined.insert(0, '/');
    }
    match query {
        Some(q) => alloc::format!("{joined}?{q}"),
        None => joined,
    }
}

/// A header list that keeps order and the caller's spelling; names
/// compare case-insensitively.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Headers(pub Vec<(String, String)>);
impl Headers {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    /// The `index`-th value of `name` (WinHTTP/WinINet header indexes).
    pub fn nth(&self, name: &str, index: usize) -> Option<&str> {
        self.0
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .nth(index)
            .map(|(_, v)| v.as_str())
    }
    pub fn remove(&mut self, name: &str) {
        self.0.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
    }
    pub fn set(&mut self, name: &str, value: &str) {
        self.remove(name);
        self.0.push((name.to_string(), value.to_string()));
    }
    pub fn add(&mut self, name: &str, value: &str) {
        self.0.push((name.to_string(), value.to_string()));
    }
    /// Comma-joins `value` onto an existing header, or adds it.
    pub fn coalesce(&mut self, name: &str, value: &str, separator: &str) {
        match self
            .0
            .iter_mut()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
        {
            Some((_, v)) => {
                v.push_str(separator);
                v.push_str(value);
            }
            None => self.add(name, value),
        }
    }
}
/// A header line without a colon or with an invalid name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidHeader;
/// Splits header lines (`Name: value\r\n`...) as a caller adds them; lines
/// without a colon are an error.
pub fn parse_header_lines(text: &str) -> Result<Vec<(String, String)>, InvalidHeader> {
    let mut out = Vec::new();
    for line in text
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
    {
        let (name, value) = line.split_once(':').ok_or(InvalidHeader)?;
        let name = name.trim();
        if name.is_empty()
            || name
                .bytes()
                .any(|b| b <= b' ' || b >= 0x7f || b"()<>@,;:\\\"/[]?={}".contains(&b))
        {
            return Err(InvalidHeader);
        }
        out.push((name.to_string(), value.trim().to_string()));
    }
    Ok(out)
}

/// A request head: `METHOD path HTTP/1.x`, Host first, then the headers.
pub fn request_head(
    method: &str,
    target: &str,
    version: &str,
    host: &str,
    headers: &Headers,
) -> Vec<u8> {
    let mut s = alloc::format!("{method} {target} {version}\r\n");
    if headers.get("Host").is_none() {
        s.push_str(&alloc::format!("Host: {host}\r\n"));
    }
    for (name, value) in &headers.0 {
        s.push_str(name);
        s.push_str(": ");
        s.push_str(value);
        s.push_str("\r\n");
    }
    s.push_str("\r\n");
    s.into_bytes()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// `HTTP/1.1`.
    pub version: String,
    pub status: u16,
    pub reason: String,
    pub headers: Headers,
    /// The status line and headers as received, CRLF-separated, ending with
    /// the empty line (WINHTTP_QUERY_RAW_HEADERS_CRLF).
    pub raw: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Not an HTTP response, or a malformed head or chunk.
    Invalid,
    /// The head is larger than the limit.
    TooLarge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Framing {
    Length(u64),
    Chunked,
    UntilClose,
    None,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Head,
    Body,
    ChunkSize,
    ChunkData(u64),
    ChunkEnd,
    Trailers,
    Done,
}

/// Incremental response parser: feed received bytes, take the head once,
/// then body bytes until `finished`.
pub struct Parser {
    buffer: Vec<u8>,
    state: State,
    framing: Framing,
    /// The request was HEAD (no body whatever the headers say).
    head_request: bool,
    response: Option<Response>,
    /// Interim (1xx) responses seen.
    pub interim: Vec<u16>,
    body: Vec<u8>,
    max_head: usize,
    /// Bytes of trailer fields read so far.
    trailer_bytes: usize,
}
/// Interim (1xx) responses one request accepts.
pub const MAX_INTERIM: usize = 16;
impl Parser {
    pub fn new(head_request: bool) -> Self {
        Self {
            buffer: Vec::new(),
            state: State::Head,
            framing: Framing::None,
            head_request,
            response: None,
            interim: Vec::new(),
            body: Vec::new(),
            max_head: 64 * 1024,
            trailer_bytes: 0,
        }
    }
    /// Feeds bytes from the connection.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), ParseError> {
        self.buffer.extend_from_slice(bytes);
        self.advance()
    }
    /// The connection closed: a read-until-close body ends; anything else
    /// unfinished is an error.
    pub fn close(&mut self) -> Result<(), ParseError> {
        match self.state {
            State::Done => Ok(()),
            State::Body if self.framing == Framing::UntilClose => {
                self.state = State::Done;
                Ok(())
            }
            _ => Err(ParseError::Invalid),
        }
    }
    pub fn response(&self) -> Option<&Response> {
        self.response.as_ref()
    }
    /// Takes up to `max` decoded body bytes received so far.
    pub fn take_body(&mut self, max: usize) -> Vec<u8> {
        let n = max.min(self.body.len());
        self.body.drain(..n).collect()
    }
    pub fn body_available(&self) -> usize {
        self.body.len()
    }
    pub fn finished(&self) -> bool {
        self.state == State::Done
    }
    /// The body ends only when the connection closes.
    pub fn until_close(&self) -> bool {
        self.framing == Framing::UntilClose
    }
    /// The connection may carry another request after this one.
    pub fn reusable(&self) -> bool {
        let Some(r) = &self.response else {
            return false;
        };
        let close = r
            .headers
            .get("Connection")
            .is_some_and(|c| c.split(',').any(|t| t.trim().eq_ignore_ascii_case("close")));
        let keep = r.headers.get("Connection").is_some_and(|c| {
            c.split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("keep-alive"))
        });
        self.state == State::Done
            && self.framing != Framing::UntilClose
            && !close
            && (r.version != "HTTP/1.0" || keep)
    }
    fn advance(&mut self) -> Result<(), ParseError> {
        loop {
            match self.state {
                State::Head => {
                    let Some(end) = find(&self.buffer, b"\r\n\r\n") else {
                        if self.buffer.len() > self.max_head {
                            return Err(ParseError::TooLarge);
                        }
                        return Ok(());
                    };
                    let head: Vec<u8> = self.buffer.drain(..end + 4).collect();
                    let text = String::from_utf8_lossy(&head).into_owned();
                    let response = parse_head(&text)?;
                    if (100..200).contains(&response.status) && response.status != 101 {
                        // A server that never stops sending interim
                        // responses is answered like an oversized head.
                        if self.interim.len() >= MAX_INTERIM {
                            return Err(ParseError::TooLarge);
                        }
                        self.interim.push(response.status);
                        continue;
                    }
                    let no_body = self.head_request
                        || response.status == 204
                        || response.status == 304
                        || (100..200).contains(&response.status);
                    self.framing = if no_body {
                        Framing::None
                    } else if response.headers.get("Transfer-Encoding").is_some_and(|t| {
                        t.rsplit(',')
                            .next()
                            .is_some_and(|l| l.trim().eq_ignore_ascii_case("chunked"))
                    }) {
                        Framing::Chunked
                    } else if let Some(length) = response.headers.get("Content-Length") {
                        Framing::Length(length.trim().parse().map_err(|_| ParseError::Invalid)?)
                    } else {
                        Framing::UntilClose
                    };
                    self.response = Some(response);
                    self.state = match self.framing {
                        Framing::None | Framing::Length(0) => State::Done,
                        Framing::Chunked => State::ChunkSize,
                        _ => State::Body,
                    };
                }
                State::Body => {
                    if self.buffer.is_empty() {
                        return Ok(());
                    }
                    match &mut self.framing {
                        Framing::Length(left) => {
                            let n = (*left).min(self.buffer.len() as u64) as usize;
                            self.body.extend(self.buffer.drain(..n));
                            *left -= n as u64;
                            if *left == 0 {
                                self.state = State::Done;
                            }
                        }
                        _ => self.body.append(&mut self.buffer),
                    }
                }
                State::ChunkSize => {
                    let Some(end) = find(&self.buffer, b"\r\n") else {
                        if self.buffer.len() > 1024 {
                            return Err(ParseError::Invalid);
                        }
                        return Ok(());
                    };
                    let line: Vec<u8> = self.buffer.drain(..end + 2).collect();
                    let text =
                        core::str::from_utf8(&line[..end]).map_err(|_| ParseError::Invalid)?;
                    let size = text.split(';').next().unwrap_or("").trim();
                    let size = u64::from_str_radix(size, 16).map_err(|_| ParseError::Invalid)?;
                    self.state = if size == 0 {
                        State::Trailers
                    } else {
                        State::ChunkData(size)
                    };
                }
                State::ChunkData(left) => {
                    if self.buffer.is_empty() {
                        return Ok(());
                    }
                    let n = left.min(self.buffer.len() as u64) as usize;
                    self.body.extend(self.buffer.drain(..n));
                    let left = left - n as u64;
                    self.state = if left == 0 {
                        State::ChunkEnd
                    } else {
                        State::ChunkData(left)
                    };
                }
                State::ChunkEnd => {
                    if self.buffer.len() < 2 {
                        return Ok(());
                    }
                    if &self.buffer[..2] != b"\r\n" {
                        return Err(ParseError::Invalid);
                    }
                    self.buffer.drain(..2);
                    self.state = State::ChunkSize;
                }
                State::Trailers => {
                    // Trailer fields count against the head's limit.
                    let Some(end) = find(&self.buffer, b"\r\n") else {
                        if self.buffer.len() > self.max_head {
                            return Err(ParseError::TooLarge);
                        }
                        return Ok(());
                    };
                    self.trailer_bytes += end + 2;
                    if self.trailer_bytes > self.max_head {
                        return Err(ParseError::TooLarge);
                    }
                    let line: Vec<u8> = self.buffer.drain(..end + 2).collect();
                    if end == 0 {
                        self.state = State::Done;
                    } else if let (Some(r), Ok(text)) =
                        (self.response.as_mut(), core::str::from_utf8(&line[..end]))
                        && let Some((name, value)) = text.split_once(':')
                    {
                        r.headers.add(name.trim(), value.trim());
                    }
                }
                State::Done => return Ok(()),
            }
        }
    }
}
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
fn parse_head(text: &str) -> Result<Response, ParseError> {
    let mut lines = text.split("\r\n");
    let status_line = lines.next().ok_or(ParseError::Invalid)?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/") {
        return Err(ParseError::Invalid);
    }
    let status = parts
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|s| (100..1000).contains(s))
        .ok_or(ParseError::Invalid)?;
    let reason = parts.next().unwrap_or("").to_string();
    let mut headers = Headers::default();
    for line in lines.filter(|l| !l.is_empty()) {
        // Obsolete line folding joins onto the previous value.
        if line.starts_with([' ', '\t']) {
            if let Some((_, v)) = headers.0.last_mut() {
                v.push(' ');
                v.push_str(line.trim());
                continue;
            }
            return Err(ParseError::Invalid);
        }
        let (name, value) = line.split_once(':').ok_or(ParseError::Invalid)?;
        headers.add(name.trim_end(), value.trim());
    }
    Ok(Response {
        version: version.to_string(),
        status,
        reason,
        headers,
        raw: text.to_string(),
    })
}

/// The method a redirect turns `method` into: 301/302/303 turn POST into
/// GET (303 everything but HEAD), 307/308 keep it.
pub fn redirect_method(status: u16, method: &str) -> Option<String> {
    match status {
        301 | 302 if method.eq_ignore_ascii_case("POST") => Some(String::from("GET")),
        303 if !method.eq_ignore_ascii_case("HEAD") => Some(String::from("GET")),
        301 | 302 | 303 | 307 | 308 => Some(method.to_string()),
        _ => None,
    }
}

/// The standard Base64 alphabet (Basic authentication).
pub fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(TABLE[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// RFC 1123 dates (`Sun, 06 Nov 1994 08:49:37 GMT`) <-> Unix seconds.
pub fn http_date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400);
    let (year, month, day) = civil(days);
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    alloc::format!(
        "{}, {day:02} {} {year:04} {:02}:{:02}:{:02} GMT",
        WEEKDAYS[days.rem_euclid(7) as usize],
        MONTHS[(month - 1) as usize],
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
/// Days since 1970-01-01 to (year, month, day) (Howard Hinnant's algorithm).
pub fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
/// (year, month, day) to days since 1970-01-01.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
pub fn parse_http_date(text: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let text = text.trim();
    let (_, rest) = text.split_once(',')?;
    let mut words = rest.split_whitespace();
    let day: u32 = words.next()?.parse().ok()?;
    let name = words.next()?.get(..3)?;
    let month = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(name))? as u32 + 1;
    let mut year: i64 = words.next()?.parse().ok()?;
    if year < 100 {
        year += if year < 70 { 2000 } else { 1900 };
    }
    let mut clock = words.next()?.split(':').map(|v| v.parse::<i64>().ok());
    let (h, m, s) = (clock.next()??, clock.next()??, clock.next()??);
    // SYSTEMTIME's range; a hostile year or field is no date at all
    // (found by `cargo xtask fuzz http-text`: the product overflowed).
    if !(1601..=30827).contains(&year)
        || !(1..=31).contains(&day)
        || !(0..24).contains(&h)
        || !(0..60).contains(&m)
        || !(0..=60).contains(&s)
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + s)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod hostile_tests {
    use super::*;
    #[test]
    fn endless_trailers_and_interim_responses_are_refused() {
        let mut p = Parser::new(false);
        p.feed(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n")
            .unwrap();
        let line = [b'x'; 1024];
        let mut refused = false;
        for _ in 0..80 {
            if p.feed(&line).is_err() {
                refused = true;
                break;
            }
        }
        assert!(refused, "a trailer line without an end");
        let mut p = Parser::new(false);
        p.feed(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n")
            .unwrap();
        let mut refused = false;
        for _ in 0..4096 {
            if p.feed(b"X-Trailer: yyyyyyyyyyyyyyyy\r\n").is_err() {
                refused = true;
                break;
            }
        }
        assert!(refused, "trailer fields without an end");
        let mut p = Parser::new(false);
        let mut refused = false;
        for _ in 0..MAX_INTERIM + 1 {
            if p.feed(b"HTTP/1.1 102 Processing\r\n\r\n").is_err() {
                refused = true;
            }
        }
        assert!(refused, "interim responses without an end");
    }
}
