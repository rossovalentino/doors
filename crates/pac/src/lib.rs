//! Proxy auto-config (PAC) scripts: the JavaScript subset proxy scripts
//! are written in, enough to run `FindProxyForURL(url, host)` with the
//! Netscape/Microsoft helper functions (`isPlainHostName`, `dnsDomainIs`,
//! `shExpMatch`, `isInNet`, `dnsResolve`, `myIpAddress`, the `Ex` forms,
//! `weekdayRange`/`dateRange`/`timeRange`, ...). A script is hostile input:
//! the interpreter bounds its source, steps, call depth, strings and arrays,
//! and every failure is an error, never a panic.
#![no_std]
extern crate alloc;
mod lex;
mod parse;
mod run;
pub use run::Value;

use alloc::{string::String, vec::Vec};

/// Bytes of script accepted.
pub const MAX_SCRIPT: usize = 512 * 1024;
/// Evaluation steps one call may take (loops included).
pub const MAX_STEPS: u64 = 2_000_000;
/// Nested calls.
pub const MAX_DEPTH: usize = 32;
/// Characters in one string, and elements in one array.
pub const MAX_STRING: usize = 64 * 1024;
pub const MAX_ARRAY: usize = 4096;

/// Why a script could not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Too large, or not this JavaScript (line).
    Syntax(u32),
    /// No `FindProxyForURL` (or `FindProxyForURLEx`).
    NoEntry,
    /// A run-time failure: an unknown name, a call of a non-function, a
    /// limit exceeded.
    Runtime(&'static str),
}

/// What the helper functions ask of the host.
pub trait Host {
    /// IPv4 addresses of `host` (dnsResolve, isResolvable, isInNet).
    fn resolve(&mut self, host: &str) -> Vec<[u8; 4]>;
    /// IPv6 addresses too (the `Ex` helpers); IPv4 ones by default.
    fn resolve_ex(&mut self, host: &str) -> Vec<String> {
        self.resolve(host).iter().map(|a| dotted(*a)).collect()
    }
    /// This host's IPv4 address (myIpAddress).
    fn my_address(&mut self) -> [u8; 4];
    /// All of this host's addresses (myIpAddressEx).
    fn my_addresses_ex(&mut self) -> Vec<String> {
        let a = self.my_address();
        alloc::vec![dotted(a)]
    }
    /// Local time: (year, month 1-12, day 1-31, weekday 0=Sunday, hour,
    /// minute, second); `utc` for the GMT forms.
    fn now(&mut self, utc: bool) -> (i32, u32, u32, u32, u32, u32, u32);
}

/// A loaded script.
pub struct Script {
    program: parse::Program,
}
impl Script {
    pub fn parse(source: &str) -> Result<Self, Error> {
        if source.len() > MAX_SCRIPT {
            return Err(Error::Syntax(0));
        }
        let tokens = lex::tokens(source)?;
        let program = parse::program(&tokens)?;
        if !program.has("FindProxyForURL") && !program.has("FindProxyForURLEx") {
            return Err(Error::NoEntry);
        }
        Ok(Self { program })
    }
    /// The script's answer for `url` (whose host is `host`): "DIRECT",
    /// "PROXY host:port; ...", as the script returns it. The `Ex` entry
    /// point is preferred when the script defines it, as Windows does.
    pub fn find_proxy(&self, url: &str, host: &str, h: &mut dyn Host) -> Result<String, Error> {
        let entry = if self.program.has("FindProxyForURLEx") {
            "FindProxyForURLEx"
        } else {
            "FindProxyForURL"
        };
        let value = run::call(&self.program, entry, &[url, host], h)?;
        Ok(value.to_text())
    }
}

/// One proxy-list entry of a script's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Direct,
    /// `PROXY` / `HTTP` / `HTTPS` host:port.
    Proxy(String),
    /// `SOCKS` / `SOCKS4` / `SOCKS5` host:port.
    Socks(String),
}
/// The routes of an answer such as "PROXY a:80; SOCKS b:1080; DIRECT", in
/// order; unknown entries are skipped.
pub fn routes(answer: &str) -> Vec<Route> {
    answer
        .split(';')
        .filter_map(|entry| {
            let mut words = entry.split_whitespace();
            let kind = words.next()?.to_ascii_uppercase();
            let target = words.next().map(String::from);
            match (kind.as_str(), target) {
                ("DIRECT", _) => Some(Route::Direct),
                ("PROXY" | "HTTP" | "HTTPS", Some(t)) => Some(Route::Proxy(t)),
                ("SOCKS" | "SOCKS4" | "SOCKS5", Some(t)) => Some(Route::Socks(t)),
                _ => None,
            }
        })
        .collect()
}

pub fn dotted(a: [u8; 4]) -> String {
    use core::fmt::Write;
    let mut s = String::new();
    let _ = write!(s, "{}.{}.{}.{}", a[0], a[1], a[2], a[3]);
    s
}
/// A dotted IPv4 address.
pub fn parse_dotted(text: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = text.split('.');
    for slot in &mut out {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}
/// `shExpMatch`: `*` any run, `?` one character, everything else itself.
pub fn shell_match(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    // Iterative glob with one backtrack point per star.
    let (mut ti, mut pi) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            ti += 1;
            pi += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests;
