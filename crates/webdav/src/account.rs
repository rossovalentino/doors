//! The NoneDrive account (`%LOCALAPPDATA%\NoneDrive\account`), written by
//! Settings and read by the sync host: the folder's address, the user name
//! and app password, the pinned certificate of a self-signed server, this
//! PC's name for conflict copies, and whether sync is paused. The file sits
//! in the user's own profile, readable by the user's processes only (as
//! Credential Manager's vault is per user); an app password is used so it
//! can be revoked on the server without changing the account password.
use alloc::{format, string::String, vec::Vec};

pub const HEADER: &str = "NoneDrive account 1";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub address: String,
    pub user: String,
    pub password: String,
    /// SHA-256 of the confirmed self-signed certificate.
    pub pinned: Option<[u8; 32]>,
    /// This PC's name (`name-DEVICE.ext` conflict copies).
    pub device: String,
    pub paused: bool,
}

fn line_safe(s: &str) -> bool {
    !s.contains(['\n', '\r'])
}
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
fn unhex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}
/// The fingerprint as Windows' certificate dialog shows a thumbprint:
/// upper-case byte pairs separated by spaces.
pub fn fingerprint(sha256: &[u8; 32]) -> String {
    let mut out = String::new();
    for (i, b) in sha256.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{b:02X}"));
    }
    out
}
impl Account {
    /// None when a field holds a line break (the file would be ambiguous).
    pub fn encode(&self) -> Option<String> {
        let fields = [&self.address, &self.user, &self.password, &self.device];
        if !fields.iter().all(|f| line_safe(f)) {
            return None;
        }
        Some(format!(
            "{HEADER}\naddress={}\nuser={}\npassword={}\npinned={}\ndevice={}\npaused={}\n",
            self.address,
            self.user,
            self.password,
            self.pinned.map(|p| hex(&p)).unwrap_or_default(),
            self.device,
            u8::from(self.paused)
        ))
    }
    pub fn decode(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != HEADER {
            return None;
        }
        let mut a = Self::default();
        let mut seen: Vec<&str> = Vec::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let (key, value) = line.split_once('=')?;
            if seen.contains(&key) {
                return None;
            }
            seen.push(key);
            match key {
                "address" => a.address = value.into(),
                "user" => a.user = value.into(),
                "password" => a.password = value.into(),
                "pinned" if value.is_empty() => a.pinned = None,
                "pinned" => a.pinned = Some(unhex32(value)?),
                "device" => a.device = value.into(),
                "paused" => {
                    a.paused = match value {
                        "0" => false,
                        "1" => true,
                        _ => return None,
                    }
                }
                // Later versions' keys are kept out, not refused.
                _ => (),
            }
        }
        (!a.address.is_empty()).then_some(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_account_round_trips() {
        let a = Account {
            address: "https://nas.home/dav/NoneDrive/".into(),
            user: "me".into(),
            password: "abcd-efgh = ijkl".into(),
            pinned: Some([0xAB; 32]),
            device: "DOORS-PC".into(),
            paused: true,
        };
        let text = a.encode().unwrap();
        assert_eq!(Account::decode(&text), Some(a.clone()));
        let mut bad = a.clone();
        bad.password = "a\nb".into();
        assert_eq!(bad.encode(), None);
        assert_eq!(Account::decode("NoneDrive account 1\nuser=x\n"), None);
        assert_eq!(
            Account::decode("NoneDrive account 1\naddress=a\naddress=b\n"),
            None
        );
        assert_eq!(
            Account::decode("NoneDrive account 1\naddress=a\npinned=xyz\n"),
            None
        );
        assert_eq!(
            fingerprint(&[0x0F; 32]).split(' ').next(),
            Some("0F")
        );
    }
}
