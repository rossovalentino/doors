use super::*;
use alloc::vec::Vec;

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn text(u: &[u16], (start, length): Span) -> String {
    String::from_utf16_lossy(&u[start..start + length])
}

/// Results measured with WinHttpCrackUrl on this PC (Windows 11 26H2).
#[test]
fn crack_matches_windows() {
    let u = units("https://user:pw@Example.COM:8443/a/b?c=d#frag");
    let c = crack(&u).unwrap();
    assert_eq!((c.scheme_number, c.port), (scheme::HTTPS, 8443));
    assert_eq!(
        [
            text(&u, c.scheme),
            text(&u, c.host),
            text(&u, c.user),
            text(&u, c.password),
            text(&u, c.path),
            text(&u, c.extra)
        ],
        ["https", "Example.COM", "user", "pw", "/a/b", "?c=d#frag"]
    );
    let u = units("http://example.com");
    let c = crack(&u).unwrap();
    assert_eq!(
        (c.port, text(&u, c.path), text(&u, c.extra)),
        (80, String::new(), String::new())
    );
    let u = units("http://h?x=1");
    let c = crack(&u).unwrap();
    assert_eq!(
        (text(&u, c.path), text(&u, c.extra)),
        (String::new(), String::from("?x=1"))
    );
    let u = units("ftp://example.com/");
    let c = crack(&u).unwrap();
    assert_eq!(
        (c.scheme_number, c.port, text(&u, c.path)),
        (scheme::FTP, 0, String::from("/"))
    );
    assert_eq!(
        crack(&units("example.com")),
        Err(error::UNRECOGNIZED_SCHEME)
    );
    assert_eq!(crack(&units("http://h:0/")).unwrap().port, 0);
    assert_eq!(crack(&units("http://:80/")), Err(error::INVALID_URL));
    assert_eq!(crack(&units("HTTP://H:65536/")), Err(error::INVALID_URL));
    assert_eq!(
        crack(&units("gopher://h/")),
        Err(error::UNRECOGNIZED_SCHEME)
    );
}

#[test]
fn create_matches_windows() {
    let plain = Parts {
        scheme: "https",
        number: scheme::HTTPS,
        host: "example.com",
        port: 443,
        user: "",
        password: "",
        path: "/p",
        extra: "?q",
    };
    assert_eq!(create(&plain), "https://example.com/p?q");
    assert_eq!(
        create(&Parts {
            port: 8443,
            user: "u",
            password: "p",
            ..plain
        }),
        "https://u:p@example.com:8443/p?q"
    );
}
