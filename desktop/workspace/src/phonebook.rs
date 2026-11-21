//! The user's VPN connections on disk, where Windows keeps the ones
//! Settings creates: `%APPDATA%\Microsoft\Network\Connections\Pbk\
//! rasphone.pbk`, one `[name]` section each. Doors' WireGuard connections
//! keep their wg-quick text there (`DoorsWireGuard=`, base64) beside the
//! keys Windows writes for every entry. Settings writes it when a
//! connection is saved or removed; the desktop shell hands the entries to
//! door.network at sign-in, so connections outlive a restart.
use alloc::{format, string::String, vec::Vec};
use door_service_client::{Disposition, Error, READ, SHARE_ALL, Storage, WRITE};
use door_service_protocol::Code;

/// The phonebook under a profile directory (`/users/<uid>`).
pub fn path(home: &str) -> String {
    format!("{home}/AppData/Roaming/Microsoft/Network/Connections/Pbk/rasphone.pbk")
}
/// The folders a first write creates, outermost first.
const FOLDERS: [&str; 6] = [
    "AppData",
    "AppData/Roaming",
    "AppData/Roaming/Microsoft",
    "AppData/Roaming/Microsoft/Network",
    "AppData/Roaming/Microsoft/Network/Connections",
    "AppData/Roaming/Microsoft/Network/Connections/Pbk",
];
/// A phonebook larger than this is not read (a damaged file).
const MAX_BYTES: u64 = 256 * 1024;

/// The phonebook's text (empty when there is none).
pub fn read(storage: &mut Storage, home: &str) -> Result<String, Error> {
    let file = match storage.open(&path(home), Disposition::OpenExisting, READ, SHARE_ALL) {
        Ok(f) => f,
        Err(Error::Service(Code::NotFound)) => return Ok(String::new()),
        Err(e) => return Err(e),
    };
    let result = (|| {
        let info = storage.info(&file)?;
        if info.bytes > MAX_BYTES {
            return Err(Error::Malformed);
        }
        let mut bytes = Vec::new();
        let mut chunk = [0; door_service_protocol::CHUNK_BYTES];
        while (bytes.len() as u64) < info.bytes {
            let count = (info.bytes as usize - bytes.len()).min(chunk.len());
            let got = storage.read(&file, bytes.len() as u64, &mut chunk[..count])?;
            if got == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..got]);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    })();
    let closed = storage.close(file);
    let text = result?;
    closed?;
    Ok(text)
}
/// Store connection `name` (`config` None: remove it) in the user's
/// phonebook.
pub fn store(
    storage: &mut Storage,
    home: &str,
    name: &str,
    config: Option<&str>,
) -> Result<(), Error> {
    let text = update(&read(storage, home)?, name, config);
    for folder in FOLDERS {
        match storage.mkdir(&format!("{home}/{folder}")) {
            Ok(()) | Err(Error::Service(Code::Exists)) => (),
            Err(e) => return Err(e),
        }
    }
    let file = storage.open(
        &path(home),
        Disposition::OpenAlways,
        READ | WRITE,
        SHARE_ALL,
    )?;
    let result = storage
        .truncate(&file, 0)
        .and_then(|()| storage.write_all(&file, 0, text.as_bytes()))
        .and_then(|()| storage.flush());
    let closed = storage.close(file);
    result?;
    closed
}
const KEY: &str = "DoorsWireGuard=";
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
fn encode64(data: &[u8]) -> String {
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
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
fn decode64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut bits = 0u32;
    let mut count = 0;
    for c in text.trim_end_matches('=').bytes() {
        let v = B64.iter().position(|&b| b == c)? as u32;
        bits = bits << 6 | v;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Some(out)
}
/// The WireGuard connections in a phonebook's text: (name, wg-quick text).
pub fn entries(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix('[')
            && let Some(end) = rest.rfind(']')
        {
            name = Some(String::from(&rest[..end]));
            continue;
        }
        if let (Some(n), Some(value)) = (name.as_ref(), line.strip_prefix(KEY))
            && let Some(bytes) = decode64(value.trim())
            && let Ok(config) = String::from_utf8(bytes)
        {
            out.push((n.clone(), config));
        }
    }
    out
}
/// The phonebook `text` with connection `name` set to `config` (None:
/// removed); other entries, Windows' own included, are kept as they are.
pub fn update(text: &str, name: &str, config: Option<&str>) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix('[')
            && let Some(end) = rest.rfind(']')
        {
            skipping = rest[..end].eq_ignore_ascii_case(name);
        }
        if !skipping {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    if let Some(config) = config {
        if !out.is_empty() && !out.ends_with("\r\n\r\n") {
            out.push_str("\r\n");
        }
        out.push_str(&format!(
            "[{name}]\r\nEncoding=1\r\nPBVersion=8\r\nType=2\r\nDevice=WireGuard Tunnel\r\n{KEY}{}\r\n\r\n",
            encode64(config.as_bytes())
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entries_survive_updates() {
        let windows = "[Work]\r\nEncoding=1\r\nType=2\r\nVpnStrategy=7\r\n\r\n";
        let one = update(
            windows,
            "Office",
            Some("[Interface]\nAddress = 10.8.0.2/24\n"),
        );
        let two = update(&one, "Home [2]", Some("[Interface]\n"));
        assert_eq!(
            entries(&two),
            [
                (
                    String::from("Office"),
                    String::from("[Interface]\nAddress = 10.8.0.2/24\n")
                ),
                (String::from("Home [2]"), String::from("[Interface]\n")),
            ]
        );
        assert!(two.starts_with(windows));
        let replaced = update(&two, "office", Some("x"));
        assert_eq!(entries(&replaced).len(), 2);
        let removed = update(&replaced, "Home [2]", None);
        // Saved again under another case: the entry takes that spelling.
        assert_eq!(
            entries(&removed),
            [(String::from("office"), String::from("x"))]
        );
        assert!(removed.contains("[Work]"));
        assert_eq!(decode64(&encode64(b"ab")), Some(b"ab".to_vec()));
        assert_eq!(decode64(&encode64(b"abc1")), Some(b"abc1".to_vec()));
    }
}
