//! Proxy configuration as data, shared by WinHTTP and WinINet: the host a
//! proxy auto-config script is asked about, the proxies its answer names,
//! and the calendar its time helpers see.
use alloc::{string::String, vec::Vec};

/// (year, month, day, weekday 0=Sunday, hour, minute, second) of a Unix
/// time.
pub fn civil(unix: i64) -> (i32, u32, u32, u32, u32, u32, u32) {
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    let weekday = (days + 4).rem_euclid(7) as u32;
    (
        year,
        month,
        day,
        weekday,
        (seconds / 3600) as u32,
        (seconds / 60 % 60) as u32,
        (seconds % 60) as u32,
    )
}

/// The host of a URL of any scheme (no user, port or brackets).
pub fn url_host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if let Some(inner) = host.strip_prefix('[') {
        return String::from(inner.split(']').next().unwrap_or(""));
    }
    String::from(host.split(':').next().unwrap_or(""))
}

/// The proxies an answer names before its first DIRECT, as WinHTTP
/// reports them ("a:80;b:80"); None for DIRECT. SOCKS entries are not
/// HTTP proxies and are skipped.
pub fn proxy_list(answer: &str) -> Option<String> {
    let mut list: Vec<String> = Vec::new();
    for route in door_pac::routes(answer) {
        match route {
            door_pac::Route::Direct => break,
            door_pac::Route::Proxy(p) => list.push(p),
            door_pac::Route::Socks(_) => (),
        }
    }
    (!list.is_empty()).then(|| list.join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn civil_dates_and_url_hosts() {
        assert_eq!(civil(0), (1970, 1, 1, 4, 0, 0, 0));
        // Sunday 8 November 2026, 14:30:05 UTC.
        assert_eq!(civil(1_794_148_205), (2026, 11, 8, 0, 14, 30, 5));
        assert_eq!(
            url_host("http://user:pw@Host.Example:8080/x?y"),
            "Host.Example"
        );
        assert_eq!(url_host("https://[fe80::1]:443/"), "fe80::1");
        assert_eq!(url_host("ftp://files.example.com"), "files.example.com");
        assert_eq!(
            proxy_list("PROXY a:1; SOCKS s:2; PROXY b:3; DIRECT; PROXY c:4"),
            Some(String::from("a:1;b:3"))
        );
        assert_eq!(proxy_list("DIRECT; PROXY a:1"), None);
    }
}
