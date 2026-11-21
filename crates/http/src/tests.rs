use super::*;
use alloc::string::ToString;

#[test]
fn urls_crack_like_windows() {
    let u = parse_url("HTTPS://user:pw@Example.COM:8443/a/b?c=d#frag").unwrap();
    assert_eq!(
        (
            u.scheme,
            u.user.as_str(),
            u.password.as_str(),
            u.host.as_str(),
            u.port
        ),
        (Scheme::Https, "user", "pw", "Example.COM", 8443)
    );
    assert_eq!((u.path.as_str(), u.fragment.as_str()), ("/a/b?c=d", "frag"));
    assert_eq!(u.host_header(), "Example.COM:8443");
    let plain = parse_url("http://example.com").unwrap();
    assert_eq!(
        (plain.port, plain.path.as_str(), plain.host_header()),
        (80, "/", "example.com".to_string())
    );
    assert_eq!(parse_url("http://h?x=1").unwrap().path, "/?x=1");
    assert_eq!(parse_url("http://[::1]:81/").unwrap().host, "::1");
    assert_eq!(
        parse_url("ftp://example.com/"),
        Err(UrlError::UnrecognizedScheme)
    );
    for bad in [
        "example.com",
        "http://",
        "http://:80/",
        "http://h:0/",
        "http://h:99999/",
        "://h",
    ] {
        assert_eq!(parse_url(bad), Err(UrlError::Invalid), "{bad}");
    }
    assert_eq!(
        parse_url("http://u@h:8080/p").unwrap().to_text(),
        "http://u@h:8080/p"
    );
}

#[test]
fn redirects_resolve_per_rfc3986() {
    let base = parse_url("http://a/b/c/d;p?q").unwrap();
    // RFC 3986 5.4.1 normal examples.
    for (reference, want) in [
        ("g", "http://a/b/c/g"),
        ("./g", "http://a/b/c/g"),
        ("g/", "http://a/b/c/g/"),
        ("/g", "http://a/g"),
        ("//g", "http://g/"),
        ("?y", "http://a/b/c/d;p?y"),
        ("g?y", "http://a/b/c/g?y"),
        ("#s", "http://a/b/c/d;p?q#s"),
        ("g#s", "http://a/b/c/g#s"),
        ("..", "http://a/b/"),
        ("../", "http://a/b/"),
        ("../g", "http://a/b/g"),
        ("../..", "http://a/"),
        ("../../g", "http://a/g"),
        ("https://other/x", "https://other/x"),
    ] {
        assert_eq!(
            resolve(&base, reference).unwrap().to_text(),
            want,
            "{reference}"
        );
    }
    assert_eq!(redirect_method(302, "POST").as_deref(), Some("GET"));
    assert_eq!(redirect_method(307, "POST").as_deref(), Some("POST"));
    assert_eq!(redirect_method(303, "PUT").as_deref(), Some("GET"));
    assert_eq!(redirect_method(303, "HEAD").as_deref(), Some("HEAD"));
    assert_eq!(redirect_method(200, "GET"), None);
}

#[test]
fn heads_and_header_lines() {
    let mut h = Headers::default();
    h.add("Accept", "*/*");
    h.coalesce("Accept", "text/html", ", ");
    assert_eq!(h.get("accept"), Some("*/*, text/html"));
    let head = request_head("GET", "/x", "HTTP/1.1", "example.com", &h);
    assert_eq!(
        head,
        b"GET /x HTTP/1.1\r\nHost: example.com\r\nAccept: */*, text/html\r\n\r\n"
    );
    assert_eq!(
        parse_header_lines("A: 1\r\nB:2\r\n\r\n").unwrap(),
        [("A".into(), "1".into()), ("B".into(), "2".into())]
    );
    assert!(parse_header_lines("no colon\r\n").is_err());
    assert!(parse_header_lines("Bad Name: x\r\n").is_err());
    assert_eq!(
        base64(b"Aladdin:open sesame"),
        "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
    );
    assert_eq!(base64(b"ab"), "YWI=");
}

#[test]
fn responses_parse_every_framing() {
    // Content-Length, fed a byte at a time.
    let mut p = Parser::new(false);
    for b in b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-A: 1\r\nX-A: 2\r\n\r\nhelloEXTRA" {
        p.feed(&[*b]).unwrap();
    }
    let r = p.response().unwrap();
    assert_eq!(
        (r.status, r.reason.as_str(), r.headers.nth("x-a", 1)),
        (200, "OK", Some("2"))
    );
    assert!(r.raw.starts_with("HTTP/1.1 200 OK\r\n") && r.raw.ends_with("\r\n\r\n"));
    assert!(p.finished() && p.reusable());
    assert_eq!(p.take_body(100), b"hello");
    // Chunked with an extension and a trailer, after a 100 Continue.
    let mut p = Parser::new(false);
    p.feed(b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;x=y\r\nWiki\r\n5\r\npedia\r\n0\r\nExpires: never\r\n\r\n").unwrap();
    assert_eq!(p.interim, [100]);
    assert!(p.finished());
    assert_eq!(p.take_body(100), b"Wikipedia");
    assert_eq!(p.response().unwrap().headers.get("Expires"), Some("never"));
    // Until close (HTTP/1.0, no length).
    let mut p = Parser::new(false);
    p.feed(b"HTTP/1.0 200 OK\r\n\r\nall").unwrap();
    p.feed(b" of it").unwrap();
    assert!(!p.finished() && p.until_close());
    p.close().unwrap();
    assert!(p.finished() && !p.reusable());
    assert_eq!(p.take_body(3), b"all");
    assert_eq!(p.body_available(), 6);
    // HEAD and 204/304 have no body.
    let mut p = Parser::new(true);
    p.feed(b"HTTP/1.1 200 OK\r\nContent-Length: 99\r\n\r\n")
        .unwrap();
    assert!(p.finished());
    let mut p = Parser::new(false);
    p.feed(b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n")
        .unwrap();
    assert!(p.finished() && !p.reusable());
    // Malformed.
    assert_eq!(
        Parser::new(false).feed(b"SSH-2.0-OpenSSH\r\n\r\n"),
        Err(ParseError::Invalid)
    );
    assert_eq!(
        Parser::new(false).feed(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n"),
        Err(ParseError::Invalid)
    );
    let mut p = Parser::new(false);
    p.feed(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort")
        .unwrap();
    assert_eq!(p.close(), Err(ParseError::Invalid));
}

#[test]
fn dates_round_trip() {
    assert_eq!(http_date(784_111_777), "Sun, 06 Nov 1994 08:49:37 GMT");
    assert_eq!(
        parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
        Some(784_111_777)
    );
    assert_eq!(parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT"), None);
    // Found by `cargo xtask fuzz http-text`: out-of-range fields.
    assert_eq!(
        parse_http_date("Sun, 06 Nov 1994294967296 08:49:37 GMT"),
        None
    );
    assert_eq!(
        parse_http_date("Sun, 06 Nov 1994 08:49:99999999999999 GMT"),
        None
    );
    assert_eq!(parse_http_date("Sun, 99 Nov 1994 08:49:37 GMT"), None);
    assert_eq!(civil(0), (1970, 1, 1));
    assert_eq!(days_from_civil(2000, 3, 1), 11_017);
}
