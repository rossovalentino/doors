use super::*;
use alloc::{string::String, vec, vec::Vec};

struct Fake {
    lookups: Vec<String>,
}
impl Host for Fake {
    fn resolve(&mut self, host: &str) -> Vec<[u8; 4]> {
        self.lookups.push(String::from(host));
        match host {
            "intranet.doors.test" => vec![[10, 0, 2, 50]],
            "www.example.com" => vec![[93, 184, 216, 34]],
            _ => Vec::new(),
        }
    }
    fn my_address(&mut self) -> [u8; 4] {
        [10, 0, 2, 15]
    }
    fn now(&mut self, utc: bool) -> (i32, u32, u32, u32, u32, u32, u32) {
        // Sunday 8 November 2026, 14:30:05 local, 13:30:05 UTC.
        (2026, 11, 8, 0, if utc { 13 } else { 14 }, 30, 5)
    }
}
fn fake() -> Fake {
    Fake {
        lookups: Vec::new(),
    }
}
fn answer(script: &str, url: &str, host: &str) -> Result<String, Error> {
    Script::parse(script)?.find_proxy(url, host, &mut fake())
}

const CORPORATE: &str = r#"
// A typical corporate script.
function FindProxyForURL(url, host) {
    var lower = host.toLowerCase();
    if (isPlainHostName(lower) || dnsDomainIs(lower, ".doors.test") ||
        localHostOrDomainIs(lower, "printer.doors.test"))
        return "DIRECT";
    if (isInNet(dnsResolve(host), "10.0.0.0", "255.0.0.0"))
        return "DIRECT";
    if (shExpMatch(url, "https://*.secure.example/*")) {
        return "PROXY secure.doors.test:8443; DIRECT";
    }
    if (url.substring(0, 4) == "ftp:")
        return "PROXY ftp-proxy.doors.test:2121";
    return "PROXY proxy.doors.test:8080; PROXY backup.doors.test:8080; DIRECT";
}
"#;

#[test]
fn a_corporate_script_routes_by_host_and_url() {
    let cases = [
        ("http://printer/", "printer", "DIRECT"),
        ("http://a.doors.test/x", "a.doors.test", "DIRECT"),
        (
            "http://intranet.doors.test/",
            "intranet.doors.test",
            "DIRECT",
        ),
        (
            "https://x.secure.example/y",
            "x.secure.example",
            "PROXY secure.doors.test:8443; DIRECT",
        ),
        (
            "ftp://files.example.com/",
            "files.example.com",
            "PROXY ftp-proxy.doors.test:2121",
        ),
        (
            "http://www.example.com/",
            "www.example.com",
            "PROXY proxy.doors.test:8080; PROXY backup.doors.test:8080; DIRECT",
        ),
    ];
    for (url, host, want) in cases {
        assert_eq!(answer(CORPORATE, url, host).unwrap(), want, "{url}");
    }
    assert_eq!(
        routes("PROXY a:80; SOCKS b:1080;DIRECT; BOGUS"),
        vec![
            Route::Proxy(String::from("a:80")),
            Route::Socks(String::from("b:1080")),
            Route::Direct
        ]
    );
}

#[test]
fn javascript_semantics_scripts_rely_on() {
    let script = r#"
        var table = ["alpha", "beta", "gamma"];
        var counter = 0;
        function pick(i) { counter++; return table[i % table.length]; }
        function FindProxyForURL(url, host) {
            var total = 0;
            for (var i = 0; i < 5; i++) { if (i == 3) continue; total += i; }
            var j = 10;
            while (j > 0) { j -= 3; if (j < 2) break; }
            var parts = host.split(".");
            var joined = parts.join("-") + "/" + parts.length;
            var t = typeof missing + "," + typeof 1 + "," + typeof "x" + "," + (null == undefined);
            var n = parseInt("0x1F") + parseInt("12px") + Number("3") + ("5" * "2");
            var s = "a" + 1 + 2 + "|" + (1 + 2);
            var cond = host.indexOf("doors") >= 0 ? "yes" : "no";
            var f = function (x) { return x * 2; };
            return [total, j, joined, t, n, s, cond, f(21), pick(4), counter,
                    "abc".charAt(1), "Hello".toUpperCase(), (7 % 3), (-7 >> 1), (1 << 4),
                    "x.y.z".lastIndexOf("."), "path/to".substr(-2), "  t  ".trim()].join(";");
        }
    "#;
    assert_eq!(
        answer(script, "http://a/", "www.doors.test").unwrap(),
        "7;1;www-doors-test/3;undefined,number,string,true;56;a12|3;yes;42;beta;1;b;HELLO;1;-4;16;3;to;t"
    );
}

#[test]
fn the_ex_helpers_and_time_ranges() {
    let script = r#"
        function FindProxyForURLEx(url, host) {
            return [isInNetEx("10.0.2.50", "10.0.0.0/8"), isResolvableEx(host), myIpAddressEx(),
                    sortIpAddressList("10.0.0.9;10.0.0.1;9.9.9.9"), getClientVersion(),
                    weekdayRange("SUN"), weekdayRange("MON", "FRI"), weekdayRange("SAT", "MON"),
                    timeRange(14), timeRange(13, "GMT"), timeRange(9, 17), timeRange(14, 0, 14, 29),
                    dateRange(8), dateRange("NOV"), dateRange(2026), dateRange(1, "NOV", 30, "NOV"),
                    dateRange("DEC", "JAN"), new Date().getHours(), new Date().getUTCHours()].join(",");
        }
        function FindProxyForURL(url, host) { return "never used"; }
    "#;
    assert_eq!(
        answer(script, "http://intranet.doors.test/", "intranet.doors.test").unwrap(),
        "true,true,10.0.2.15,9.9.9.9;10.0.0.1;10.0.0.9,1.0,true,false,true,true,true,true,false,true,true,true,true,false,14,13"
    );
}

#[test]
fn hostile_scripts_fail_without_hanging_or_panicking() {
    let forever = "function FindProxyForURL(u, h) { while (true) {} }";
    assert_eq!(
        answer(forever, "u", "h"),
        Err(Error::Runtime("too many steps"))
    );
    let recursion = "function f() { return f(); } function FindProxyForURL(u, h) { return f(); }";
    assert_eq!(
        answer(recursion, "u", "h"),
        Err(Error::Runtime("calls nested too deeply"))
    );
    let growth = "function FindProxyForURL(u, h) { var s = 'xxxxxxxx'; while (true) s = s + s; }";
    assert_eq!(
        answer(growth, "u", "h"),
        Err(Error::Runtime("string too long"))
    );
    let nested = alloc::format!(
        "function FindProxyForURL(u, h) {{ return {}1{}; }}",
        "(".repeat(5000),
        ")".repeat(5000)
    );
    assert!(matches!(answer(&nested, "u", "h"), Err(Error::Syntax(_))));
    assert_eq!(answer("var x = 1;", "u", "h"), Err(Error::NoEntry));
    assert!(matches!(
        answer("function FindProxyForURL( {", "u", "h"),
        Err(Error::Syntax(_))
    ));
    assert_eq!(
        answer(
            "function FindProxyForURL(u, h) { return nothing(); }",
            "u",
            "h"
        ),
        Err(Error::Runtime("unknown name"))
    );
    // Arbitrary bytes never panic.
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..2000 {
        let mut text = String::new();
        let alphabet = b"function FindProxyForURL(u,h){return \"DIRECT\"+[1,2].join(';');}var x=1;while(x<3)x++;'\\\"/*-+!?:<>=&|%";
        for _ in 0..(seed % 120) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            text.push(alphabet[(seed % alphabet.len() as u64) as usize] as char);
        }
        let _ = answer(&text, "http://a/", "a");
    }
}

#[test]
fn shell_patterns_and_dotted_addresses() {
    assert!(shell_match(
        "http://www.doors.test/a",
        "http://*.doors.test/*"
    ));
    assert!(shell_match("abc", "a?c"));
    assert!(!shell_match("abc", "a?d"));
    assert!(shell_match("", "*"));
    assert!(shell_match("a*b", "a*b"));
    assert!(!shell_match("ab", "abc"));
    assert_eq!(parse_dotted("10.0.2.15"), Some([10, 0, 2, 15]));
    for bad in ["10.0.2", "10.0.2.256", "a.b.c.d", "1.2.3.4.5", "1..2.3"] {
        assert_eq!(parse_dotted(bad), None, "{bad}");
    }
}

#[test]
fn the_limits_fit_a_small_thread_stack() {
    extern crate std;
    // The deepest a script may nest, parsed and run on a small stack
    // (256 KiB in a release build, a quarter of a Windows thread's).
    let handle = std::thread::Builder::new()
        .stack_size(if cfg!(debug_assertions) { 4 << 20 } else { 256 << 10 })
        .spawn(|| {
            let deep = alloc::format!(
                "function FindProxyForURL(u, h) {{ return {}1{}; }}",
                "(".repeat(25),
                ")".repeat(25)
            );
            assert_eq!(answer(&deep, "u", "h").unwrap(), "1");
            let recursion =
                "function f(n) { return n == 0 ? 0 : 1 + f(n - 1); } function FindProxyForURL(u, h) { return f(30); }";
            assert_eq!(answer(recursion, "u", "h").unwrap(), "30");
            let too_deep = alloc::format!(
                "function FindProxyForURL(u, h) {{ return {}1{}; }}",
                "(".repeat(5000),
                ")".repeat(5000)
            );
            assert!(answer(&too_deep, "u", "h").is_err());
        })
        .unwrap();
    handle.join().unwrap();
}
