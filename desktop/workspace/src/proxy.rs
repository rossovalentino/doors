//! The manual proxy server (Settings > Network & internet > Proxy), kept
//! where Windows keeps it: `HKCU\Software\Microsoft\Windows\CurrentVersion\
//! Internet Settings` with `ProxyEnable` (DWORD), `ProxyServer`
//! ("host:port", or per-protocol "https=host:port;http=...") and
//! `ProxyOverride` ("<local>;*.example.com"). HTTPS fetches tunnel through
//! it with HTTP CONNECT, as WinINet/WinHTTP do (`https::Fetch`).
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use door_service_client::{Error, Storage};
use door_service_client::{RegistryRoot, RegistryValue, RegistryView};

pub const KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub host: String,
    pub port: u16,
    /// ProxyOverride entries (`<local>` = names without a dot).
    pub bypass: Vec<String>,
}
impl Proxy {
    /// "host:port" as ProxyServer stores it.
    pub fn server(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
    /// Whether `host` connects directly (ProxyOverride).
    pub fn bypassed(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.bypass.iter().any(|entry| {
            let e = entry.trim().to_ascii_lowercase();
            if e == "<local>" {
                return !host.contains('.') || host == "localhost" || host.starts_with("127.");
            }
            crate::proxy::matches(&e, &host)
        })
    }
}
/// `*` wildcard matching for ProxyOverride entries.
fn matches(pattern: &str, host: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let h: Vec<char> = host.chars().collect();
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < h.len() {
        if i < p.len() && (p[i] == h[j] || p[i] == '?') {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}
/// ProxyServer text: "host:port" or "http=a:1;https=b:2" (HTTPS wins);
/// a missing port is 80, as WinINet assumes.
pub fn parse_server(text: &str) -> Option<(String, u16)> {
    let text = text.trim();
    let pick = if text.contains('=') {
        let parts: Vec<&str> = text.split(';').collect();
        let find = |scheme: &str| {
            parts
                .iter()
                .find_map(|p| p.trim().strip_prefix(scheme).map(str::trim))
        };
        find("https=").or_else(|| find("http="))?
    } else {
        text
    };
    let pick = pick
        .strip_prefix("http://")
        .unwrap_or(pick)
        .trim_end_matches('/');
    let (host, port) = match pick.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok().filter(|p| *p > 0)?),
        None => (pick, 80),
    };
    let valid = !host.is_empty()
        && host.len() <= 64
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    valid.then(|| (String::from(host), port))
}
fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}
fn string_value(text: &str) -> Result<RegistryValue, Error> {
    let mut bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    bytes.extend_from_slice(&[0, 0]);
    RegistryValue::new(1, &bytes)
}
fn read_string(value: &RegistryValue) -> String {
    let data = value.data();
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}
/// The signed-in user's proxy, None when off or unset.
pub fn read(storage: &mut Storage) -> Option<Proxy> {
    let key = storage
        .registry_open(
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &units(KEY),
            1,
            false,
        )
        .ok()?;
    let enabled = storage
        .registry_query(&key, &units("ProxyEnable"))
        .ok()
        .is_some_and(|v| v.kind == 4 && v.data() != [0, 0, 0, 0]);
    let server = storage
        .registry_query(&key, &units("ProxyServer"))
        .ok()
        .filter(|v| v.kind == 1)
        .map(|v| read_string(&v));
    let bypass = storage
        .registry_query(&key, &units("ProxyOverride"))
        .ok()
        .filter(|v| v.kind == 1)
        .map(|v| read_string(&v))
        .unwrap_or_default();
    let _ = storage.registry_close(key);
    if !enabled {
        return None;
    }
    let (host, port) = parse_server(&server?)?;
    Some(Proxy {
        host,
        port,
        bypass: bypass
            .split(';')
            .filter(|s| !s.trim().is_empty())
            .map(String::from)
            .collect(),
    })
}
/// Save (or, with None, switch off) the user's proxy.
pub fn write(storage: &mut Storage, proxy: Option<&Proxy>) -> Result<(), Error> {
    // Create the key path one level at a time (a fresh profile may lack it).
    let mut path = String::new();
    let mut key = None;
    for part in KEY.split('\\') {
        if !path.is_empty() {
            path.push('\\');
        }
        path.push_str(part);
        if let Some(k) = key.take() {
            let _ = storage.registry_close(k);
        }
        key = Some(storage.registry_open(
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &units(&path),
            3,
            true,
        )?);
    }
    let key = key.ok_or(Error::Malformed)?;
    let result: Result<(), Error> = (|| {
        storage.registry_set(
            &key,
            &units("ProxyEnable"),
            RegistryValue::dword(u32::from(proxy.is_some())),
        )?;
        if let Some(p) = proxy {
            storage.registry_set(&key, &units("ProxyServer"), string_value(&p.server())?)?;
            storage.registry_set(
                &key,
                &units("ProxyOverride"),
                string_value(&p.bypass.join(";"))?,
            )?;
        }
        Ok(())
    })();
    let _ = storage.registry_close(key);
    result?;
    storage.flush()
}

/// `Internet Settings\Connections`, where DefaultConnectionSettings keeps
/// the "Automatically detect settings" switch (flags at offset 8, bit 0x8).
pub const CONNECTIONS: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings\\Connections";
const AUTO_DETECT: u32 = 0x8;
/// Open (creating) a key path under HKCU one level at a time.
fn open_path(
    storage: &mut Storage,
    path_text: &str,
    create: bool,
) -> Result<door_service_client::Key, Error> {
    let mut path = String::new();
    let mut key = None;
    for part in path_text.split('\\') {
        if !path.is_empty() {
            path.push('\\');
        }
        path.push_str(part);
        if let Some(k) = key.take() {
            let _ = storage.registry_close(k);
        }
        key = Some(storage.registry_open(
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &units(&path),
            if create { 3 } else { 1 },
            create,
        )?);
    }
    key.ok_or(Error::Malformed)
}
/// "Automatically detect settings" and the setup script address.
pub fn read_automatic(storage: &mut Storage) -> (bool, Option<String>) {
    let detect = open_path(storage, CONNECTIONS, false)
        .ok()
        .and_then(|key| {
            let value = storage
                .registry_query(&key, &units("DefaultConnectionSettings"))
                .ok();
            let _ = storage.registry_close(key);
            value
        })
        .filter(|v| v.kind == 3 && v.data().len() >= 12)
        // A user who never changed it has detection on.
        .is_none_or(|v| {
            let d = v.data();
            u32::from_le_bytes([d[8], d[9], d[10], d[11]]) & AUTO_DETECT != 0
        });
    let script = open_path(storage, KEY, false).ok().and_then(|key| {
        let value = storage
            .registry_query(&key, &units("AutoConfigURL"))
            .ok()
            .filter(|v| v.kind == 1)
            .map(|v| read_string(&v))
            .filter(|s| !s.is_empty());
        let _ = storage.registry_close(key);
        value
    });
    (detect, script)
}
/// DefaultConnectionSettings with "Automatically detect settings" set to
/// `on` (a new blob as Windows writes one when there is none: version
/// 0x46, a change counter, the flags, empty proxy, bypass and script).
pub fn connection_settings(old: Option<&[u8]>, on: bool) -> Vec<u8> {
    let mut blob = match old {
        Some(b) if b.len() >= 12 => b.to_vec(),
        _ => {
            let mut b = alloc::vec![0u8; 12 + 12 + 32];
            b[0] = 0x46;
            b[8] = 0x1;
            b
        }
    };
    let counter = u32::from_le_bytes([blob[4], blob[5], blob[6], blob[7]]).wrapping_add(1);
    blob[4..8].copy_from_slice(&counter.to_le_bytes());
    let mut flags = u32::from_le_bytes([blob[8], blob[9], blob[10], blob[11]]);
    if on {
        flags |= AUTO_DETECT;
    } else {
        flags &= !AUTO_DETECT;
    }
    blob[8..12].copy_from_slice(&flags.to_le_bytes());
    blob
}
pub fn write_auto_detect(storage: &mut Storage, on: bool) -> Result<(), Error> {
    let key = open_path(storage, CONNECTIONS, true)?;
    let old = storage
        .registry_query(&key, &units("DefaultConnectionSettings"))
        .ok()
        .filter(|v| v.kind == 3)
        .map(|v| v.data().to_vec());
    let blob = connection_settings(old.as_deref(), on);
    let result = RegistryValue::new(3, &blob)
        .and_then(|v| storage.registry_set(&key, &units("DefaultConnectionSettings"), v));
    let _ = storage.registry_close(key);
    result?;
    storage.flush()
}
/// Save (or, with None, remove) the setup script address (AutoConfigURL).
pub fn write_script(storage: &mut Storage, script: Option<&str>) -> Result<(), Error> {
    let key = open_path(storage, KEY, true)?;
    let result = match script {
        Some(url) => {
            string_value(url).and_then(|v| storage.registry_set(&key, &units("AutoConfigURL"), v))
        }
        None => match storage.registry_delete_value(&key, &units("AutoConfigURL")) {
            Ok(()) | Err(Error::Service(door_service_protocol::Code::NotFound)) => Ok(()),
            Err(e) => Err(e),
        },
    };
    let _ = storage.registry_close(key);
    result?;
    storage.flush()
}
/// A setup script address Settings accepts: http(s), a host, at most 2048
/// characters.
pub fn valid_script(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.len() <= 2048
        && (lower.starts_with("http://") || lower.starts_with("https://"))
        && text.split_once("://").is_some_and(|(_, rest)| {
            !rest.is_empty() && !rest.starts_with('/') && !rest.contains(char::is_whitespace)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_detection_switch_lives_in_the_connection_blob() {
        let fresh = connection_settings(None, true);
        assert_eq!(fresh[0], 0x46);
        assert_eq!(&fresh[4..8], &[1, 0, 0, 0]);
        assert_eq!(fresh[8], 0x9);
        let off = connection_settings(Some(&fresh), false);
        assert_eq!(&off[4..8], &[2, 0, 0, 0]);
        assert_eq!(off[8], 0x1);
        assert_eq!(off.len(), fresh.len());
        assert!(valid_script("http://wpad.example.com/proxy.pac"));
        assert!(valid_script("HTTPS://10.0.2.2:8080/x.pac"));
        for bad in [
            "ftp://x/y.pac",
            "http://",
            "http:///x",
            "proxy.pac",
            "http://a b/x",
        ] {
            assert!(!valid_script(bad), "{bad}");
        }
    }
    #[test]
    fn server_text_and_overrides() {
        assert_eq!(
            parse_server("10.0.2.2:10090"),
            Some(("10.0.2.2".into(), 10090))
        );
        assert_eq!(
            parse_server("http=proxy:8080;https=secure:3128"),
            Some(("secure".into(), 3128))
        );
        assert_eq!(
            parse_server("http://corp-proxy:8080/"),
            Some(("corp-proxy".into(), 8080))
        );
        assert_eq!(parse_server("proxy"), Some(("proxy".into(), 80)));
        assert_eq!(parse_server("bad host:1"), None);
        assert_eq!(parse_server("h:0"), None);
        let p = Proxy {
            host: "10.0.2.2".into(),
            port: 10090,
            bypass: alloc::vec!["<local>".into(), "*.corp.example".into(), "10.*".into()],
        };
        assert!(p.bypassed("intranet"));
        assert!(p.bypassed("localhost"));
        assert!(p.bypassed("wiki.corp.example"));
        assert!(p.bypassed("10.1.2.3"));
        assert!(!p.bypassed("api.open-meteo.com"));
        assert_eq!(p.server(), "10.0.2.2:10090");
    }
}
