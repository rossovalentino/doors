//! The cookie store of a WinHTTP session or of WinINet (RFC 6265 subset).
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

/// One stored cookie (RFC 6265 subset: domain/path matching, Secure,
/// Max-Age/Expires).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Lower case; `host_only` when no Domain attribute was given.
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    /// Unix expiry; None for a session cookie.
    pub expires: Option<i64>,
}
#[derive(Clone, Debug, Default)]
pub struct Jar(pub Vec<Cookie>);
fn default_path(request_path: &str) -> String {
    let path = request_path.split(['?', '#']).next().unwrap_or("/");
    match path.rfind('/') {
        Some(0) | None => String::from("/"),
        Some(i) => path[..i].to_string(),
    }
}
fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain
        || (host.ends_with(domain) && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
}
fn path_matches(request: &str, cookie: &str) -> bool {
    let request = request.split(['?', '#']).next().unwrap_or("/");
    request == cookie
        || (request.starts_with(cookie)
            && (cookie.ends_with('/') || request.as_bytes().get(cookie.len()) == Some(&b'/')))
}
impl Jar {
    /// Stores a Set-Cookie value received for `host`/`path` at `now`.
    pub fn store(&mut self, header: &str, host: &str, path: &str, now: i64) {
        let mut parts = header.split(';');
        let Some((name, value)) = parts.next().and_then(|p| p.split_once('=')) else {
            return;
        };
        let host = host.to_ascii_lowercase();
        let mut cookie = Cookie {
            name: name.trim().to_string(),
            value: value.trim().to_string(),
            domain: host.clone(),
            host_only: true,
            path: default_path(path),
            secure: false,
            expires: None,
        };
        if cookie.name.is_empty() {
            return;
        }
        let mut max_age = None;
        for attribute in parts {
            let (key, value) = attribute
                .split_once('=')
                .map_or((attribute.trim(), ""), |(k, v)| (k.trim(), v.trim()));
            match key.to_ascii_lowercase().as_str() {
                "domain" if !value.is_empty() => {
                    let domain = value.trim_start_matches('.').to_ascii_lowercase();
                    // A cookie for another site is ignored.
                    if !domain_matches(&host, &domain) {
                        return;
                    }
                    cookie.domain = domain;
                    cookie.host_only = false;
                }
                "path" if value.starts_with('/') => cookie.path = value.to_string(),
                "secure" => cookie.secure = true,
                "max-age" => max_age = value.parse::<i64>().ok(),
                "expires" => {
                    if cookie.expires.is_none() {
                        cookie.expires = door_http::parse_http_date(value);
                    }
                }
                _ => (),
            }
        }
        if let Some(seconds) = max_age {
            cookie.expires = Some(now.saturating_add(seconds));
        }
        self.0.retain(|c| {
            !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
        });
        if cookie.expires.is_none_or(|e| e > now) {
            self.0.push(cookie);
        }
    }
    /// Drops the session cookies (no expiry), as ending a browser session
    /// does.
    pub fn end_session(&mut self) {
        self.0.retain(|c| c.expires.is_some());
    }
    /// The Cookie header for a request, most specific path first.
    pub fn header(&self, host: &str, path: &str, secure: bool, now: i64) -> Option<String> {
        let host = host.to_ascii_lowercase();
        let mut matching: Vec<&Cookie> = self
            .0
            .iter()
            .filter(|c| {
                if c.host_only {
                    c.domain == host
                } else {
                    domain_matches(&host, &c.domain)
                }
            })
            .filter(|c| {
                path_matches(path, &c.path)
                    && (secure || !c.secure)
                    && c.expires.is_none_or(|e| e > now)
            })
            .collect();
        if matching.is_empty() {
            return None;
        }
        matching.sort_by_key(|c| core::cmp::Reverse(c.path.len()));
        Some(
            matching
                .iter()
                .map(|c| alloc::format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cookies_follow_rfc6265() {
        let mut jar = Jar::default();
        jar.store(
            "sid=1; Path=/; HttpOnly",
            "Shop.Example.com",
            "/cart/view",
            100,
        );
        jar.store(
            "pref=dark; Domain=.example.com; Path=/",
            "shop.example.com",
            "/",
            100,
        );
        jar.store("tok=x; Secure; Max-Age=10", "shop.example.com", "/a/b", 100);
        jar.store("evil=1; Domain=other.com", "shop.example.com", "/", 100);
        assert_eq!(
            jar.header("shop.example.com", "/cart", false, 105)
                .as_deref(),
            Some("sid=1; pref=dark")
        );
        assert_eq!(
            jar.header("www.example.com", "/", false, 105).as_deref(),
            Some("pref=dark")
        );
        // `tok` is Secure, path /a, and expires at 110.
        assert_eq!(
            jar.header("shop.example.com", "/a/c", true, 105).as_deref(),
            Some("tok=x; sid=1; pref=dark")
        );
        assert_eq!(
            jar.header("shop.example.com", "/a/c", true, 111).as_deref(),
            Some("sid=1; pref=dark")
        );
        // Max-Age=0 deletes.
        jar.store("sid=gone; Path=/; Max-Age=0", "shop.example.com", "/", 120);
        assert_eq!(
            jar.header("shop.example.com", "/", false, 121).as_deref(),
            Some("pref=dark")
        );
        assert!(jar.header("other.com", "/", false, 121).is_none());
    }
    #[test]
    fn ending_the_session_keeps_persistent_cookies() {
        let mut jar = Jar::default();
        jar.store("a=1", "h", "/", 0);
        jar.store("b=2; Max-Age=100", "h", "/", 0);
        jar.end_session();
        assert_eq!(jar.header("h", "/", false, 1).as_deref(), Some("b=2"));
    }
}
