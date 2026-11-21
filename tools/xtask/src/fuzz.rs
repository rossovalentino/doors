//! `cargo xtask fuzz [target] [--iterations N] [--seed S]`: mutation fuzzing
//! of the parsers that read what other hosts and downloaded files send
//! (HTTP responses and authentication challenges, TLS records and
//! certificates, PAC scripts, FTP replies and listings, the URL cache index,
//! firewall rules, WireGuard configurations and datagrams, Windows Installer
//! databases, cabinets, Authenticode signatures, PE images and icons). Each
//! target starts from valid seeds and applies random mutations (bit flips,
//! boundary bytes, inserted, deleted, duplicated and spliced ranges, long
//! numbers); a panic, or an input that takes longer than a second, fails
//! the run and the input is kept under `out/fuzz/<target>/`. Deterministic:
//! the same seed replays the same inputs.
use crate::evidence::Run;
use serde_json::json;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    time::{Duration, Instant},
};

/// xorshift64*: small, fast, reproducible.
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

const INTERESTING: [u8; 16] = [
    0, 0xff, 0x7f, 0x80, b'\n', b'\r', b'|', b'=', b';', b'"', b'\\', b'%', b'{', b'}', b':', b' ',
];
const NUMBERS: [&[u8]; 8] = [
    b"0",
    b"-1",
    b"65536",
    b"4294967295",
    b"4294967296",
    b"18446744073709551616",
    b"99999999999999999999999999",
    b"-2147483649",
];

/// One mutated input from `seeds`.
pub fn mutate(rng: &mut Rng, seeds: &[Vec<u8>]) -> Vec<u8> {
    let mut data = seeds[rng.below(seeds.len())].clone();
    for _ in 0..1 + rng.below(8) {
        let len = data.len();
        match rng.below(9) {
            0 if len > 0 => {
                let at = rng.below(len);
                data[at] ^= 1 << rng.below(8);
            }
            1 if len > 0 => {
                let at = rng.below(len);
                data[at] = INTERESTING[rng.below(INTERESTING.len())];
            }
            2 => {
                let at = rng.below(len + 1);
                let n = 1 + rng.below(16);
                let bytes: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                data.splice(at..at, bytes);
            }
            3 if len > 0 => {
                let at = rng.below(len);
                let end = (at + 1 + rng.below(32)).min(len);
                data.drain(at..end);
            }
            4 if len > 0 => {
                // A range repeated: long lines, deep nesting.
                let at = rng.below(len);
                let end = (at + 1 + rng.below(64)).min(len);
                let piece = data[at..end].to_vec();
                let times = 1 + rng.below(64);
                let mut copy = Vec::with_capacity(piece.len() * times);
                for _ in 0..times {
                    copy.extend_from_slice(&piece);
                }
                data.splice(at..at, copy);
            }
            5 => {
                // Splice with another seed.
                let other = &seeds[rng.below(seeds.len())];
                if !other.is_empty() {
                    let from = rng.below(other.len());
                    let to = rng.below(len + 1);
                    data.truncate(to);
                    data.extend_from_slice(&other[from..]);
                }
            }
            6 if len > 0 => {
                // A digit run replaced by a hostile number.
                let at = rng.below(len);
                if data[at].is_ascii_digit() {
                    let mut end = at;
                    while end < len && data[end].is_ascii_digit() {
                        end += 1;
                    }
                    let n = NUMBERS[rng.below(NUMBERS.len())];
                    data.splice(at..end, n.iter().copied());
                }
            }
            7 if len > 0 => {
                data.truncate(rng.below(len));
            }
            8 if len >= 4 => {
                // A little-endian length field set to a boundary.
                let at = rng.below(len - 3);
                let v: u32 = [0, 1, 0x7fff_ffff, 0xffff_ffff, 0x8000_0000, 0x10000][rng.below(6)];
                data[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
            _ => (),
        }
    }
    if data.len() > 1 << 20 {
        data.truncate(1 << 20);
    }
    data
}

pub struct Target {
    pub name: &'static str,
    pub seeds: fn() -> Vec<Vec<u8>>,
    pub run: fn(&[u8]),
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}
fn seeds(list: &[&[u8]]) -> Vec<Vec<u8>> {
    list.iter().map(|s| s.to_vec()).collect()
}
/// A resolver for PAC scripts that answers without a network.
struct Offline;
impl door_pac::Host for Offline {
    fn resolve(&mut self, host: &str) -> Vec<[u8; 4]> {
        if host.len().is_multiple_of(2) {
            vec![[10, 0, 2, 2]]
        } else {
            Vec::new()
        }
    }
    fn my_address(&mut self) -> [u8; 4] {
        [10, 0, 2, 15]
    }
    fn now(&mut self, _utc: bool) -> (i32, u32, u32, u32, u32, u32, u32) {
        (2026, 11, 8, 0, 12, 0, 0)
    }
}
struct Counter(u8);
impl door_wireguard::tunnel::Random for Counter {
    fn fill(&mut self, out: &mut [u8]) {
        for b in out {
            self.0 = self.0.wrapping_mul(29).wrapping_add(7);
            *b = self.0;
        }
    }
}

const PAC: &[u8] = b"function FindProxyForURL(url, host) {\n  if (isPlainHostName(host) || dnsDomainIs(host, \".doors.test\")) return \"DIRECT\";\n  if (shExpMatch(url, \"http://*.example/*\") && isInNet(dnsResolve(host), \"10.0.0.0\", \"255.0.0.0\")) return \"PROXY a:8080; DIRECT\";\n  var d = new Date(); if (weekdayRange(\"MON\", \"FRI\") && timeRange(8, 18)) return \"PROXY b:3128\";\n  for (var i = 0; i < 3; i++) { host = host.substring(1) + host.charAt(0); }\n  return myIpAddress() == \"10.0.2.15\" ? \"PROXY c:1\" : \"DIRECT\";\n}\n";

fn http_seeds() -> Vec<Vec<u8>> {
    seeds(&[
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: text/plain\r\n\r\nhello",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;x=y\r\nhello\r\n10\r\n0123456789abcdef\r\n0\r\nTrailer: t\r\n\r\n",
        b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 204 No Content\r\n\r\n",
        b"HTTP/1.0 301 Moved\r\nLocation: http://a/b\r\nSet-Cookie: a=b; Path=/; Max-Age=60\r\n\r\nbody until close",
        b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Digest realm=\"r\", nonce=\"n\", qop=\"auth\"\r\nContent-Length: 0\r\n\r\n",
    ])
}
fn tls_seeds() -> Vec<Vec<u8>> {
    let (_, hello) = door_tls::Client::new("example.com", 1_790_000_000, [3; 96]);
    let mut server_hello = vec![0x16, 3, 3, 0, 0, 2, 0, 0, 0, 3, 3];
    server_hello.extend_from_slice(&[0x5a; 32]);
    server_hello.push(32);
    server_hello.extend_from_slice(&[0x11; 32]);
    server_hello.extend_from_slice(&[
        0x13, 0x01, 0, 0, 0x2e, 0, 0x2b, 0, 2, 3, 4, 0, 0x33, 0, 0x24, 0, 0x1d, 0, 0x20,
    ]);
    server_hello.extend_from_slice(&[0x42; 32]);
    let body = (server_hello.len() - 5) as u16;
    server_hello[3..5].copy_from_slice(&body.to_be_bytes());
    let hs = (server_hello.len() - 9) as u32;
    server_hello[6..9].copy_from_slice(&hs.to_be_bytes()[1..]);
    vec![
        hello,
        server_hello,
        vec![0x15, 3, 3, 0, 2, 2, 40],
        vec![0x14, 3, 3, 0, 1, 1],
    ]
}
const CERTIFICATES: [&[u8]; 3] = [
    include_bytes!("../../../crates/authenticode/roots/microsoft-root-2010.der"),
    include_bytes!("../../../crates/authenticode/roots/sectigo-code-signing-root-e46.der"),
    include_bytes!("../../../crates/authenticode/roots/thawte-primary-root.der"),
];
/// A small valid PE32+ image: headers and one section.
fn pe_image() -> Vec<u8> {
    let mut f = vec![0u8; 0x400];
    f[0..2].copy_from_slice(b"MZ");
    f[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    let pe = 0x80;
    f[pe..pe + 4].copy_from_slice(b"PE\0\0");
    let coff = pe + 4;
    f[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
    f[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    f[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
    f[coff + 18..coff + 20].copy_from_slice(&0x0022u16.to_le_bytes());
    let opt = coff + 20;
    f[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
    f[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes()); // entry
    f[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
    f[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes());
    f[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes());
    f[opt + 40..opt + 42].copy_from_slice(&6u16.to_le_bytes());
    f[opt + 48..opt + 50].copy_from_slice(&6u16.to_le_bytes());
    f[opt + 56..opt + 60].copy_from_slice(&0x2000u32.to_le_bytes()); // size of image
    f[opt + 60..opt + 64].copy_from_slice(&0x200u32.to_le_bytes()); // headers
    f[opt + 68..opt + 70].copy_from_slice(&3u16.to_le_bytes()); // console
    f[opt + 70..opt + 72].copy_from_slice(&0x8160u16.to_le_bytes());
    f[opt + 72..opt + 80].copy_from_slice(&0x10_0000u64.to_le_bytes());
    f[opt + 80..opt + 88].copy_from_slice(&0x1000u64.to_le_bytes());
    f[opt + 88..opt + 96].copy_from_slice(&0x10_0000u64.to_le_bytes());
    f[opt + 96..opt + 104].copy_from_slice(&0x1000u64.to_le_bytes());
    f[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
    let section = opt + 240;
    f[section..section + 5].copy_from_slice(b".text");
    f[section + 8..section + 12].copy_from_slice(&0x10u32.to_le_bytes());
    f[section + 12..section + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    f[section + 16..section + 20].copy_from_slice(&0x200u32.to_le_bytes());
    f[section + 20..section + 24].copy_from_slice(&0x200u32.to_le_bytes());
    f[section + 36..section + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());
    f[0x200] = 0xc3;
    f
}

pub const TARGETS: &[Target] = &[
    Target {
        name: "http-response",
        seeds: http_seeds,
        run: |b| {
            // Arriving in pieces, as a connection delivers them.
            let mut p = door_http::Parser::new(b.first().is_some_and(|c| c & 1 == 1));
            for piece in b.chunks(1 + usize::from(b.last().copied().unwrap_or(0) % 64)) {
                if p.feed(piece).is_err() {
                    return;
                }
                let _ = p.take_body(1 << 16);
            }
            let _ = p.close();
            let _ = p.response();
        },
    },
    Target {
        name: "http-text",
        seeds: || {
            seeds(&[
                b"http://user:pw@example.com:8080/a/b?c=d#e",
                b"https://[::1]:443/",
                b"Content-Type: text/html; charset=utf-8\r\nX-A: b\r\n\tfolded",
                b"Sun, 06 Nov 1994 08:49:37 GMT",
                b"Sunday, 06-Nov-94 08:49:37 GMT",
            ])
        },
        run: |b| {
            let t = text(b);
            let _ = door_http::parse_url(&t);
            let _ = door_http::parse_header_lines(&t);
            let _ = door_http::parse_http_date(&t);
        },
    },
    Target {
        name: "http-auth",
        seeds: || {
            let challenge =
                door_http::auth::ntlm::server_challenge([1; 8], "DOORS", 0, 0xe288_8235);
            let mut out = seeds(&[
                b"Digest realm=\"doors\", nonce=\"abc\", qop=\"auth,auth-int\", algorithm=SHA-256, opaque=\"x\"",
                b"Negotiate TlRMTVNTUAACAAAA",
                b"Basic realm=\"r\"",
            ]);
            out.push(door_http::auth::spnego::response(Some(1), Some(&challenge)));
            out.push(door_http::auth::spnego::init(
                &door_http::auth::ntlm::negotiate(),
            ));
            out.push(challenge);
            out
        },
        run: |b| {
            let t = text(b);
            let _ = door_http::auth::digest::params(&t, "Digest");
            let _ = door_http::auth::scheme_of(&t);
            let _ = door_http::auth::base64_decode(&t);
            let _ = door_http::auth::ntlm::parse_challenge(b);
            let _ = door_http::auth::ntlm::negotiate_flags(b);
            let _ = door_http::auth::ntlm::server_verify(b, [1; 8], "password");
            let _ = door_http::auth::spnego::response_token(b);
            let _ = door_http::auth::spnego::init_token(b);
        },
    },
    Target {
        name: "tls-records",
        seeds: tls_seeds,
        run: |b| {
            let (mut client, _) = door_tls::Client::new("example.com", 1_790_000_000, [3; 96]);
            for piece in b.chunks(1 + usize::from(b.first().copied().unwrap_or(0))) {
                if client.receive(piece).is_err() {
                    return;
                }
            }
        },
    },
    Target {
        name: "x509",
        seeds: || CERTIFICATES.iter().map(|c| c.to_vec()).collect(),
        run: |b| {
            let _ = door_tls::x509::parse(b);
        },
    },
    Target {
        name: "pac",
        seeds: || {
            seeds(&[
                PAC,
                b"function FindProxyForURLEx(u, h) { return \"PROXY [::1]:8080\"; }",
            ])
        },
        run: |b| {
            let t = text(b);
            if let Ok(script) = door_pac::Script::parse(&t) {
                let _ = script.find_proxy("http://www.example/a", "www.example", &mut Offline);
            }
            let _ = door_pac::routes(&t);
            let _ = door_pac::shell_match("www.example.com", &t);
        },
    },
    Target {
        name: "ftp",
        seeds: || {
            seeds(&[
                b"220 Doors FTP\r\n230-Welcome\r\n230 Logged in\r\n257 \"/a \"\"b\"\"\" is current\r\n",
                b"227 Entering Passive Mode (10,0,2,2,195,80).\r\n229 Entering Extended Passive Mode (|||50000|)\r\n",
                b"-rw-r--r--    1 owner    group        1234 Nov 08 10:00 file.txt\r\ndrwxr-xr-x 1 o g 0 Jan 01  2026 dir\r\n11-08-26  10:00AM       <DIR>          win\r\n",
            ])
        },
        run: |b| {
            let mut replies = door_windows_inet::ftp::Replies::default();
            if replies.push(b).is_ok() {
                while let Ok(Some(_)) = replies.take() {}
            }
            let t = text(b);
            let _ = door_windows_inet::ftp::passive(&t);
            let _ = door_windows_inet::ftp::extended_passive(&t);
            let _ = door_windows_inet::ftp::quoted_path(&t);
            for line in t.lines() {
                let _ = door_windows_inet::ftp::listing_line(line);
            }
        },
    },
    Target {
        name: "url-cache-index",
        seeds: || {
            let empty = door_windows_inet::cache::Index::new(7).encode();
            let entry = format!(
                "{empty}entry\thttp://a/b?c\tABCD1234\\b[1].htm\t65\t5\t0\t0\t0\t0\t1\t0\thtm\t{}\n",
                "485454502f312e3120323030204f4b0d0a"
            );
            vec![empty.into_bytes(), entry.into_bytes()]
        },
        run: |b| {
            if let Some(index) = door_windows_inet::cache::Index::decode(&text(b)) {
                let _ = index.encode();
            }
        },
    },
    Target {
        name: "cookies-and-proxy-lists",
        seeds: || {
            seeds(&[
                b"a=b; Domain=.example.com; Path=/x; Expires=Wed, 09 Jun 2027 10:18:14 GMT; Secure; HttpOnly",
                b"c=d; Max-Age=60; Path=/",
                b"PROXY a:8080; SOCKS b:1080; DIRECT",
                b"http://user@host.example:81/path",
            ])
        },
        run: |b| {
            let t = text(b);
            let mut jar = door_windows_inet::cookies::Jar::default();
            jar.store(&t, "www.example.com", "/x/y", 1_790_000_000);
            let _ = jar.header("www.example.com", "/x/y", true, 1_790_000_001);
            let _ = door_windows_inet::proxyconfig::proxy_list(&t);
            let _ = door_windows_inet::proxyconfig::url_host(&t);
        },
    },
    Target {
        name: "firewall-rule",
        seeds: || {
            seeds(&[
                b"v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|Profile=Public|LPort=8080,9000-9010|RPort=80|RA4=10.0.0.0/8,LocalSubnet|App=C:\\a.exe|Name=App|Desc=d|EmbedCtxt=@FirewallAPI.dll,-1|",
                b"v2.10|Action=Block|Active=FALSE|Dir=Out|Protocol=17|Name=x|",
            ])
        },
        run: |b| {
            let t = text(b);
            if let Ok(rule) = door_firewall::Rule::parse("{id}", &t) {
                let _ = door_firewall::Rule::parse("{id}", &rule.text());
                let policy = door_firewall::Policy {
                    rules: vec![rule],
                    ..Default::default()
                };
                for direction in [door_firewall::Direction::In, door_firewall::Direction::Out] {
                    let _ = policy.permits(
                        &door_firewall::Connection {
                            direction,
                            protocol: 6,
                            local_port: 8080,
                            remote_address: door_firewall::Address::V4([10, 1, 2, 3]),
                            remote_port: 80,
                            remote_kind: door_firewall::keyword::LOCAL_SUBNET,
                            icmp: Some((8, 0)),
                            program: Some("c:\\A.EXE"),
                        },
                        door_firewall::profile::PUBLIC,
                    );
                }
            }
        },
    },
    Target {
        name: "wireguard-config",
        seeds: || {
            let key = door_wireguard::config::encode_key(&[9; 32]);
            vec![format!(
                "[Interface]\nPrivateKey = {key}\nAddress = 10.8.0.2/24, 10.9.0.2/32\nDNS = 10.8.0.1\nMTU = 1420\n[Peer]\nPublicKey = {key}\nPresharedKey = {key}\nAllowedIPs = 0.0.0.0/0, 10.0.0.0/8\nEndpoint = vpn.example:51820\nPersistentKeepalive = 25\n"
            )
            .into_bytes()]
        },
        run: |b| {
            let t = text(b);
            if let Ok(config) = door_wireguard::Config::parse(&t) {
                let _ = door_wireguard::Config::parse(&config.to_text());
            }
            let _ = door_wireguard::Endpoint::parse(&t);
            let _ = door_wireguard::config::decode_key(&t);
        },
    },
    Target {
        name: "wireguard-datagrams",
        seeds: wireguard_seeds,
        run: |b| {
            // Length-prefixed datagrams into a tunnel that expects the
            // seeds' peer.
            let (ours, theirs) = ([1u8; 32], [2u8; 32]);
            let mut tunnel = door_wireguard::Tunnel::new(
                ours,
                door_wireguard::public_key(&theirs),
                None,
                vec![door_wireguard::Cidr {
                    address: [0; 4],
                    prefix: 0,
                }],
                Some(25),
                1420,
            );
            let mut io = door_wireguard::Io::default();
            let mut rng = Counter(3);
            let mut rest = b;
            let mut millis = 1_000;
            while rest.len() >= 2 {
                let n = usize::from(u16::from_le_bytes([rest[0], rest[1]])).min(rest.len() - 2);
                let now = door_wireguard::Now {
                    millis,
                    unix_nanos: millis * 1_000_000,
                };
                let _ = tunnel.receive(&rest[2..2 + n], now, &mut rng, &mut io);
                tunnel.tick(now, &mut rng, &mut io);
                rest = &rest[2 + n..];
                millis += 30;
            }
        },
    },
    Target {
        name: "msi-database",
        seeds: || vec![door_msi::fixtures::companion()],
        run: |b| {
            if let Ok(database) = door_msi::Database::open(b) {
                let _ = database.tables.len();
            }
        },
    },
    Target {
        name: "cabinet",
        seeds: || {
            let mut out = Vec::new();
            for compression in [door_cab::Compression::Stored, door_cab::Compression::MsZip] {
                let mut w = door_cab::Writer::new(compression);
                w.add("a.txt", b"hello hello hello hello", 0x5b68, 0x5000, 0x20);
                w.add("b.bin", &[7; 300], 0x5b68, 0x5000, 0x20);
                out.push(w.finish());
            }
            out
        },
        run: |b| {
            if let Ok(cabinet) = door_cab::Cabinet::parse(b) {
                for file in &cabinet.files {
                    let _ = cabinet.extract(file);
                }
            }
        },
    },
    Target {
        name: "pe-image",
        seeds: || vec![pe_image()],
        run: |b| {
            let _ = door_pe_loader::BundlePlan::new(&[door_pe_loader::ModuleSource {
                name: "probe.exe",
                bytes: b,
                base: door_pe_loader::ImageBase(0x1_4000_0000),
            }]);
            let _ = door_authenticode::layout(b);
            let _ = door_authenticode::verify(b, 1_790_000_000, &[]);
            let _ = door_pe_icon::group(b);
        },
    },
    Target {
        name: "icons",
        seeds: || {
            // GRPICONDIR with one 32x32 entry, and a 2x2 32-bit DIB.
            let mut directory = vec![0, 0, 1, 0, 1, 0, 32, 32, 0, 0, 1, 0, 32, 0];
            directory.extend_from_slice(&(40u32 + 16).to_le_bytes());
            directory.extend_from_slice(&1u16.to_le_bytes());
            let mut dib = vec![0u8; 40 + 16 + 8];
            dib[0] = 40;
            dib[4] = 2;
            dib[8] = 4;
            dib[12] = 1;
            dib[14] = 32;
            vec![directory, dib]
        },
        run: |b| {
            let _ = door_pe_icon::directory(b);
            let _ = door_pe_icon::decode_dib(b);
            let _ = door_pe_icon::is_png(b);
        },
    },
];
fn wireguard_seeds() -> Vec<Vec<u8>> {
    // The real handshake from the peer's side, length-prefixed.
    let (ours, theirs) = ([1u8; 32], [2u8; 32]);
    let mut peer = door_wireguard::Tunnel::new(
        theirs,
        door_wireguard::public_key(&ours),
        None,
        vec![door_wireguard::Cidr {
            address: [0; 4],
            prefix: 0,
        }],
        None,
        1420,
    );
    let mut io = door_wireguard::Io::default();
    let mut rng = Counter(5);
    let now = door_wireguard::Now {
        millis: 1_000,
        unix_nanos: 1_000_000_000,
    };
    peer.connect(now, &mut rng, &mut io);
    let mut out = Vec::new();
    for datagram in io.network {
        out.extend_from_slice(&(datagram.len() as u16).to_le_bytes());
        out.extend_from_slice(&datagram);
    }
    vec![out]
}

/// One target for `iterations` inputs from `seed`: the failures found
/// (input, what went wrong).
pub fn fuzz(target: &Target, iterations: u64, seed: u64) -> Vec<(Vec<u8>, String)> {
    let mut rng = Rng::new(seed ^ fnv(target.name));
    let corpus = (target.seeds)();
    let mut failures = Vec::new();
    // Every seed as it is, then mutations.
    let mut inputs: Vec<Vec<u8>> = corpus.clone();
    inputs.reverse();
    for n in 0..corpus.len() as u64 + iterations {
        let input = if n < corpus.len() as u64 {
            inputs.pop().unwrap_or_default()
        } else {
            mutate(&mut rng, &corpus)
        };
        let started = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(|| (target.run)(&input)));
        let took = started.elapsed();
        match outcome {
            Err(panic) => {
                let why = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| String::from("panic"));
                failures.push((input, format!("panicked: {why}")));
            }
            Ok(()) if took > Duration::from_secs(1) => {
                failures.push((input, format!("slow: {} ms", took.as_millis())));
            }
            Ok(()) => (),
        }
        if failures.len() >= 8 {
            break;
        }
    }
    failures
}
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    })
}

/// `cargo xtask fuzz [target] [--iterations N] [--seed S]`.
pub fn run(run: &mut Run, args: &[&str]) -> Result<(), String> {
    let mut iterations = 20_000u64;
    let mut seed = 0x15_2026_1108u64;
    let mut only = None;
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--iterations" => {
                i += 1;
                iterations = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or("--iterations N")?;
            }
            "--seed" => {
                i += 1;
                seed = args.get(i).and_then(|v| v.parse().ok()).ok_or("--seed S")?;
            }
            name => only = Some(name),
        }
        i += 1;
    }
    if let Some(name) = only
        && !TARGETS.iter().any(|t| t.name == name)
    {
        let names: Vec<&str> = TARGETS.iter().map(|t| t.name).collect();
        return Err(format!(
            "no fuzz target {name}; targets: {}",
            names.join(", ")
        ));
    }
    // Panics are the findings: keep their messages off the console.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut results = Vec::new();
    let mut failed = false;
    for target in TARGETS.iter().filter(|t| only.is_none_or(|n| n == t.name)) {
        let started = Instant::now();
        let failures = fuzz(target, iterations, seed);
        let mut kept = Vec::new();
        for (input, why) in &failures {
            let directory = run.root.join("out/fuzz").join(target.name);
            std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            let file = directory.join(format!(
                "failure-{:016x}.bin",
                fnv(&String::from_utf8_lossy(input))
            ));
            std::fs::write(&file, input).map_err(|e| e.to_string())?;
            kept.push(json!({"why":why,"input":file.display().to_string(),"bytes":input.len()}));
        }
        let millis = started.elapsed().as_millis() as u64;
        println!(
            "[{}] fuzz {} {iterations} inputs in {millis} ms",
            if failures.is_empty() {
                "passed"
            } else {
                "failed"
            },
            target.name
        );
        for (_, why) in &failures {
            println!("    {why}");
        }
        failed |= !failures.is_empty();
        results.push(
            json!({"target":target.name,"iterations":iterations,"seed":seed,
            "failures":kept,"millis":millis}),
        );
    }
    std::panic::set_hook(hook);
    run.check("fuzz", json!({"targets":results}));
    if failed {
        return Err("fuzzing found failures (inputs under out/fuzz)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_target_survives_a_short_run() {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let mut all = Vec::new();
        for target in TARGETS {
            for (input, why) in fuzz(target, 300, 7) {
                all.push(format!("{}: {why} ({} bytes)", target.name, input.len()));
            }
        }
        std::panic::set_hook(hook);
        assert!(all.is_empty(), "{all:#?}");
    }
    #[test]
    fn mutation_is_reproducible() {
        let corpus = vec![b"seed input 123".to_vec()];
        let a: Vec<Vec<u8>> = (0..50)
            .scan(Rng::new(9), |r, _| Some(mutate(r, &corpus)))
            .collect();
        let b: Vec<Vec<u8>> = (0..50)
            .scan(Rng::new(9), |r, _| Some(mutate(r, &corpus)))
            .collect();
        assert_eq!(a, b);
        assert!(a.iter().any(|m| m != &corpus[0]));
    }
}
