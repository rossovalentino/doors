//! wg-quick configuration files (the format WireGuard for Windows imports
//! and exports): `[Interface]` and one `[Peer]`. Keys are case-insensitive,
//! `#` starts a comment, lists are comma separated. IPv4 and IPv6
//! addresses, AllowedIPs and DNS servers are kept apart (`addresses6`,
//! `dns6`, `allowed_ips6`); the endpoint is reached over IPv4.
use crate::ip::{Cidr, Cidr6, parse_ipv4};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// A host name or dotted IPv4 address.
    pub host: String,
    pub port: u16,
}
impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}
impl Endpoint {
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let text = text.trim();
        if text.starts_with('[') {
            return Err("IPv6 endpoints are not supported");
        }
        let (host, port) = text.rsplit_once(':').ok_or("Missing port from endpoint")?;
        let port = port
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or("Invalid port")?;
        let host = host.trim();
        let valid = !host.is_empty()
            && host.len() <= 253
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !label.starts_with('-')
                    && !label.ends_with('-')
            });
        if !valid {
            return Err("Invalid endpoint host");
        }
        Ok(Self {
            host: host.to_string(),
            port,
        })
    }
    /// The host when it is an IPv4 address.
    pub fn address(&self) -> Option<[u8; 4]> {
        parse_ipv4(&self.host)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Peer {
    pub public_key: [u8; 32],
    pub preshared_key: Option<[u8; 32]>,
    pub endpoint: Option<Endpoint>,
    pub allowed_ips: Vec<Cidr>,
    /// AllowedIPs' IPv6 prefixes.
    pub allowed_ips6: Vec<Cidr6>,
    pub persistent_keepalive: Option<u16>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub private_key: [u8; 32],
    pub addresses: Vec<Cidr>,
    pub dns: Vec<[u8; 4]>,
    /// DNS search domains (non-address DNS entries).
    pub dns_search: Vec<String>,
    pub mtu: Option<u16>,
    pub listen_port: Option<u16>,
    pub peer: Peer,
    /// The interface's IPv6 addresses and DNS servers.
    pub addresses6: Vec<Cidr6>,
    pub dns6: Vec<[u8; 16]>,
}

/// A parse failure: the 1-based line and WireGuard's wording of the problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub line: usize,
    pub message: &'static str,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            f.write_str(self.message)
        } else {
            write!(f, "Line {}: {}", self.line, self.message)
        }
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
/// A 32-byte key in standard padded base64 (44 characters).
pub fn encode_key(key: &[u8; 32]) -> String {
    let mut out = String::with_capacity(44);
    for chunk in key.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
/// A key from its base64 form; exactly 32 bytes, canonical padding.
pub fn decode_key(text: &str) -> Option<[u8; 32]> {
    let text = text.trim().as_bytes();
    if text.len() != 44 || text[43] != b'=' || text[42] == b'=' {
        return None;
    }
    let mut out = [0u8; 33];
    let mut bits = 0u32;
    let mut count = 0;
    let mut at = 0;
    for &c in &text[..43] {
        let v = B64.iter().position(|b| *b == c)? as u32;
        bits = (bits << 6) | v;
        count += 6;
        if count >= 8 {
            count -= 8;
            out[at] = (bits >> count) as u8;
            at += 1;
            bits &= (1 << count) - 1;
        }
    }
    // The two spare bits of the last character must be zero.
    (at == 32 && bits == 0).then(|| out[..32].try_into().unwrap())
}

fn key_value(line: &str) -> Option<(String, &str)> {
    let (k, v) = line.split_once('=')?;
    Some((k.trim().to_ascii_lowercase(), v.trim()))
}
fn list(value: &str) -> Result<Vec<&str>, &'static str> {
    let items: Vec<&str> = value.split(',').map(str::trim).collect();
    if items.len() > 1 && items.iter().any(|s| s.is_empty()) {
        return Err("Two commas in a row");
    }
    Ok(items.into_iter().filter(|s| !s.is_empty()).collect())
}
fn is_ipv6(text: &str) -> bool {
    text.contains(':')
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, Error> {
        #[derive(PartialEq)]
        enum Section {
            None,
            Interface,
            Peer,
        }
        let mut section = Section::None;
        let mut config = Config::default();
        let (mut has_private, mut peers, mut has_public) = (false, 0usize, false);
        for (n, raw) in text.lines().enumerate() {
            let line_no = n + 1;
            let fail = |message| Error {
                line: line_no,
                message,
            };
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            match line.to_ascii_lowercase().as_str() {
                "[interface]" => {
                    section = Section::Interface;
                    continue;
                }
                "[peer]" => {
                    peers += 1;
                    if peers > 1 {
                        return Err(fail("A Doors VPN connection has exactly one peer"));
                    }
                    section = Section::Peer;
                    continue;
                }
                _ => (),
            }
            if section == Section::None {
                return Err(fail("Line must occur in a section"));
            }
            let (key, value) =
                key_value(line).ok_or(fail("Config key is missing an equals separator"))?;
            if value.is_empty() {
                return Err(fail("Key must have a value"));
            }
            match (&section, key.as_str()) {
                (Section::Interface, "privatekey") => {
                    config.private_key =
                        decode_key(value).ok_or(fail("Keys must decode to exactly 32 bytes"))?;
                    has_private = true;
                }
                (Section::Interface, "address") => {
                    for item in list(value).map_err(fail)? {
                        if is_ipv6(item) {
                            config
                                .addresses6
                                .push(Cidr6::parse(item).ok_or(fail("Invalid IP address"))?);
                            continue;
                        }
                        config
                            .addresses
                            .push(Cidr::parse(item).ok_or(fail("Invalid IP address"))?);
                    }
                }
                (Section::Interface, "dns") => {
                    for item in list(value).map_err(fail)? {
                        if let Some(a) = parse_ipv4(item) {
                            config.dns.push(a);
                        } else if is_ipv6(item) {
                            let a: core::net::Ipv6Addr =
                                item.parse().ok().ok_or(fail("Invalid IP address"))?;
                            config.dns6.push(a.octets());
                        } else {
                            config.dns_search.push(item.to_string());
                        }
                    }
                }
                (Section::Interface, "mtu") => {
                    config.mtu = Some(
                        value
                            .parse()
                            .ok()
                            .filter(|m| (576..=65535).contains(m))
                            .ok_or(fail("Invalid MTU"))?,
                    );
                }
                (Section::Interface, "listenport") => {
                    config.listen_port = Some(value.parse().ok().ok_or(fail("Invalid port"))?);
                }
                // wg-quick extras WireGuard for Windows accepts; Doors runs no
                // scripts and keeps one routing table.
                (
                    Section::Interface,
                    "table" | "preup" | "postup" | "predown" | "postdown" | "saveconfig" | "fwmark",
                ) => {}
                (Section::Interface, _) => return Err(fail("Invalid key for [Interface] section")),
                (Section::Peer, "publickey") => {
                    config.peer.public_key =
                        decode_key(value).ok_or(fail("Keys must decode to exactly 32 bytes"))?;
                    has_public = true;
                }
                (Section::Peer, "presharedkey") => {
                    config.peer.preshared_key = Some(
                        decode_key(value).ok_or(fail("Keys must decode to exactly 32 bytes"))?,
                    );
                }
                (Section::Peer, "endpoint") => {
                    config.peer.endpoint = Some(Endpoint::parse(value).map_err(fail)?);
                }
                (Section::Peer, "allowedips") => {
                    for item in list(value).map_err(fail)? {
                        if is_ipv6(item) {
                            config
                                .peer
                                .allowed_ips6
                                .push(Cidr6::parse(item).ok_or(fail("Invalid IP address"))?);
                            continue;
                        }
                        config
                            .peer
                            .allowed_ips
                            .push(Cidr::parse(item).ok_or(fail("Invalid IP address"))?);
                    }
                }
                (Section::Peer, "persistentkeepalive") => {
                    config.peer.persistent_keepalive = if value.eq_ignore_ascii_case("off") {
                        None
                    } else {
                        let v: u16 = value
                            .parse()
                            .ok()
                            .ok_or(fail("Invalid persistent keepalive"))?;
                        (v != 0).then_some(v)
                    };
                }
                (Section::Peer, _) => return Err(fail("Invalid key for [Peer] section")),
                (Section::None, _) => unreachable!(),
            }
        }
        if !has_private {
            return Err(Error {
                line: 0,
                message: "An interface must have a private key",
            });
        }
        if peers == 0 || !has_public {
            return Err(Error {
                line: 0,
                message: "A peer must have a public key",
            });
        }
        Ok(config)
    }
    /// The wg-quick text (what Export writes).
    pub fn to_text(&self) -> String {
        let mut out = String::from("[Interface]\n");
        out += &format!("PrivateKey = {}\n", encode_key(&self.private_key));
        if !self.addresses.is_empty() || !self.addresses6.is_empty() {
            out += &format!(
                "Address = {}\n",
                join(
                    self.addresses
                        .iter()
                        .map(|c| format!("{c}"))
                        .chain(self.addresses6.iter().map(|c| format!("{c}")))
                )
            );
        }
        if !self.dns.is_empty() || !self.dns6.is_empty() || !self.dns_search.is_empty() {
            let servers = self
                .dns
                .iter()
                .map(|a| format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]))
                .chain(
                    self.dns6
                        .iter()
                        .map(|a| format!("{}", core::net::Ipv6Addr::from(*a))),
                );
            out += &format!(
                "DNS = {}\n",
                join(servers.chain(self.dns_search.iter().cloned()))
            );
        }
        if let Some(mtu) = self.mtu {
            out += &format!("MTU = {mtu}\n");
        }
        if let Some(port) = self.listen_port {
            out += &format!("ListenPort = {port}\n");
        }
        out += "\n[Peer]\n";
        out += &format!("PublicKey = {}\n", encode_key(&self.peer.public_key));
        if let Some(psk) = &self.peer.preshared_key {
            out += &format!("PresharedKey = {}\n", encode_key(psk));
        }
        if !self.peer.allowed_ips.is_empty() || !self.peer.allowed_ips6.is_empty() {
            out += &format!(
                "AllowedIPs = {}\n",
                join(
                    self.peer
                        .allowed_ips
                        .iter()
                        .map(|c| format!("{c}"))
                        .chain(self.peer.allowed_ips6.iter().map(|c| format!("{c}")))
                )
            );
        }
        if let Some(e) = &self.peer.endpoint {
            out += &format!("Endpoint = {e}\n");
        }
        if let Some(k) = self.peer.persistent_keepalive {
            out += &format!("PersistentKeepalive = {k}\n");
        }
        out
    }
    /// "Block untunneled traffic": WireGuard for Windows turns its
    /// kill-switch on when the peer routes the whole IPv4 space as one /0
    /// (unticking it rewrites that to 0.0.0.0/1 and 128.0.0.0/1).
    pub fn blocks_untunneled(&self) -> bool {
        self.peer.allowed_ips.contains(&Cidr::DEFAULT)
    }
    /// The tunnel's MTU: the configured one, else 1420 as WireGuard picks
    /// for a 1500-byte link.
    pub fn tunnel_mtu(&self) -> u16 {
        self.mtu.unwrap_or(1420)
    }
}
fn join(items: impl Iterator<Item = String>) -> String {
    let mut out = String::new();
    for (i, s) in items.enumerate() {
        if i > 0 {
            out += ", ";
        }
        out += &s;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Exported from WireGuard\n[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\nAddress = 10.8.0.2/24, fd00::2/64\nDNS = 1.1.1.1, 2606:4700::1111, corp.example\nMTU = 1380\n\n[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\nPresharedKey = FpCyhws9cxwWoV4xELtfJvjJN+zQVRPISllRWgeopVE=\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = vpn.example.com:51820\nPersistentKeepalive = 25\n";

    #[test]
    fn keys_round_trip_through_base64() {
        let key: [u8; 32] = core::array::from_fn(|i| (i * 37 + 5) as u8);
        let text = encode_key(&key);
        assert_eq!(text.len(), 44);
        assert_eq!(decode_key(&text), Some(key));
        // The whitepaper's sample keys decode.
        assert!(decode_key("yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=").is_some());
        for bad in [
            "",
            "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk",
            "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBml=",
            "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBm*=",
        ] {
            assert!(decode_key(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn parses_a_windows_export() {
        let c = Config::parse(SAMPLE).unwrap();
        assert_eq!(c.addresses, [Cidr::new([10, 8, 0, 2], 24)]);
        assert_eq!(c.dns, [[1, 1, 1, 1]]);
        assert_eq!(c.dns_search, ["corp.example"]);
        assert_eq!(c.mtu, Some(1380));
        assert_eq!(c.addresses6, [Cidr6::parse("fd00::2/64").unwrap()]);
        assert_eq!(c.dns6.len(), 1);
        assert_eq!(c.peer.allowed_ips6, [Cidr6::new([0; 16], 0)]);
        assert_eq!(
            c.peer.endpoint,
            Some(Endpoint {
                host: "vpn.example.com".into(),
                port: 51820
            })
        );
        assert_eq!(c.peer.allowed_ips, [Cidr::DEFAULT]);
        assert_eq!(c.peer.persistent_keepalive, Some(25));
        assert!(c.peer.preshared_key.is_some());
        assert!(c.blocks_untunneled());
        // Export and parse again: the same configuration, IPv6 included.
        assert_eq!(Config::parse(&c.to_text()).unwrap(), c);
    }

    #[test]
    fn reports_wireguards_errors_with_lines() {
        let cases = [
            ("PrivateKey = x", 1, "Line must occur in a section"),
            (
                "[Interface]\nPrivateKey",
                2,
                "Config key is missing an equals separator",
            ),
            ("[Interface]\nPrivateKey =", 2, "Key must have a value"),
            (
                "[Interface]\nPrivateKey = abc",
                2,
                "Keys must decode to exactly 32 bytes",
            ),
            (
                "[Interface]\nListen = 5",
                2,
                "Invalid key for [Interface] section",
            ),
            (
                "[Interface]\nAddress = 10.0.0.1/40",
                2,
                "Invalid IP address",
            ),
            (
                "[Peer]\nAllowedIPs = 1.0.0.0/8,,2.0.0.0/8",
                2,
                "Two commas in a row",
            ),
            ("[Peer]\nEndpoint = host", 2, "Missing port from endpoint"),
            ("[Peer]\nEndpoint = bad_host:1", 2, "Invalid endpoint host"),
            (
                "[Peer]\n[Peer]",
                2,
                "A Doors VPN connection has exactly one peer",
            ),
        ];
        for (text, line, message) in cases {
            assert_eq!(Config::parse(text), Err(Error { line, message }), "{text}");
        }
        let no_peer = "[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\n";
        assert_eq!(
            Config::parse(no_peer).unwrap_err().message,
            "A peer must have a public key"
        );
        assert_eq!(
            alloc::format!("{}", Config::parse("x").unwrap_err()),
            "Line 1: Line must occur in a section"
        );
    }

    #[test]
    fn a_split_default_route_does_not_block() {
        let text = SAMPLE.replace("0.0.0.0/0, ::/0", "0.0.0.0/1, 128.0.0.0/1");
        let c = Config::parse(&text).unwrap();
        assert!(!c.blocks_untunneled());
        assert_eq!(c.tunnel_mtu(), 1380);
    }
}
