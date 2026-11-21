//! Per-app notification senders (Settings > System > Notifications >
//! "Notifications from apps and other senders"). Every sender that posts
//! gets a registry key `HKCU\Software\Doors\Notifications\Senders\<key>`
//! (`notifications::sender` documents the keys and the flag layout) with
//! its display name and flags, created the first time it posts. The shell
//! keeps the flags it read here and applies them to toasts and the centre;
//! a settings change (storage's settings watch) makes it re-read the
//! entries whose registry version changed.
extern crate alloc;
use alloc::{string::String, vec::Vec};
use door_service_protocol::notifications::sender;

/// One sender the shell knows: its key, flags and the registry version of
/// its key when they were read (0: not stored yet).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known {
    pub key: String,
    pub flags: u32,
    pub version: u64,
}
#[derive(Default)]
pub struct Senders {
    pub known: Vec<Known>,
    /// Senders seen in the centre without an entry yet: (key, display name).
    pending: Vec<(String, String)>,
}
/// Senders the shell tracks at most (more than the centre can hold).
pub const MAX_SENDERS: usize = 64;
impl Senders {
    /// The stored flags of `key` (None: unknown so far).
    pub fn flags(&self, key: &str) -> Option<u32> {
        self.known.iter().find(|k| k.key == key).map(|k| k.flags)
    }
    pub fn version(&self, key: &str) -> Option<u64> {
        self.known.iter().find(|k| k.key == key).map(|k| k.version)
    }
    /// Record what the registry holds for `key`; true when the flags changed
    /// (presentation must be recomputed). Invalid flags read as the default.
    pub fn learn(&mut self, key: &str, flags: u32, version: u64) -> bool {
        let flags = if sender::valid(flags) {
            flags
        } else {
            sender::DEFAULT
        };
        self.pending.retain(|(k, _)| k != key);
        match self.known.iter_mut().find(|k| k.key == key) {
            Some(k) => {
                let changed = k.flags != flags;
                k.flags = flags;
                k.version = version;
                changed
            }
            None => {
                if self.known.len() >= MAX_SENDERS {
                    self.known.remove(0);
                }
                self.known.push(Known {
                    key: key.into(),
                    flags,
                    version,
                });
                flags != sender::DEFAULT
            }
        }
    }
    /// The sender's key is gone (deleted in the registry): it is created
    /// again, with the default flags, the next time it posts.
    pub fn forget(&mut self, key: &str) -> bool {
        let before = self.known.len();
        self.known.retain(|k| k.key != key);
        before != self.known.len()
    }
    /// A sender without an entry posted: create it (once).
    pub fn request(&mut self, key: &str, name: &str) {
        if self.flags(key).is_none()
            && !self.pending.iter().any(|(k, _)| k == key)
            && self.pending.len() < MAX_SENDERS
        {
            self.pending.push((key.into(), name.into()));
        }
    }
    pub fn take_pending(&mut self) -> Vec<(String, String)> {
        core::mem::take(&mut self.pending)
    }
    pub fn keys(&self) -> Vec<String> {
        self.known.iter().map(|k| k.key.clone()).collect()
    }
}
/// Registry path (UTF-16) of `key`'s entry.
pub fn path_units(key: &str) -> Vec<u16> {
    let mut path: Vec<u16> = sender::PATH.encode_utf16().collect();
    path.push(u16::from(b'\\'));
    path.extend(key.encode_utf16());
    path
}
/// REG_SZ bytes of a display name: at most `NAME_UNITS` UTF-16 units (a
/// surrogate pair is never split) and the terminator, 64 bytes at most.
pub fn name_value(name: &str) -> Vec<u8> {
    let mut units: Vec<u16> = Vec::new();
    for c in name.chars() {
        let mut buf = [0; 2];
        let encoded = c.encode_utf16(&mut buf);
        if units.len() + encoded.len() > sender::NAME_UNITS {
            break;
        }
        units.extend_from_slice(encoded);
    }
    units.push(0);
    units.iter().flat_map(|u| u.to_le_bytes()).collect()
}
fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Registry I/O through the user's storage connection (no key handle is
/// kept open: storage has few open-key slots).
pub mod store {
    use super::*;
    use door_service_client::{Error, RegistryRoot, RegistryValue, RegistryView, Result, Storage};
    use door_service_protocol::Code;
    const READ: u16 = 1;
    const WRITE: u16 = 2;
    fn flags_of(value: Result<RegistryValue>) -> Result<Option<u32>> {
        match value {
            Ok(v) if v.kind == 4 && v.data().len() == 4 => {
                Ok(Some(u32::from_le_bytes(v.data().try_into().unwrap())))
            }
            Ok(_) => Ok(Some(sender::DEFAULT)),
            Err(Error::Service(Code::NotFound)) => Ok(None),
            Err(e) => Err(e),
        }
    }
    /// Create `key`'s entry if missing (Name and default Flags; existing
    /// values are kept) and return its (flags, version).
    pub fn create(storage: &mut Storage, key: &str, name: &str) -> Result<(u32, u64)> {
        let k = storage.registry_open(
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &path_units(key),
            READ | WRITE,
            true,
        )?;
        let result = (|| {
            let flags = match flags_of(storage.registry_query(&k, &units(sender::FLAGS)))? {
                Some(flags) => flags,
                None => {
                    storage.registry_set(
                        &k,
                        &units(sender::FLAGS),
                        RegistryValue::dword(sender::DEFAULT),
                    )?;
                    sender::DEFAULT
                }
            };
            if let Err(Error::Service(Code::NotFound)) =
                storage.registry_query(&k, &units(sender::NAME))
            {
                storage.registry_set(
                    &k,
                    &units(sender::NAME),
                    RegistryValue::new(1, &name_value(name))?,
                )?;
            }
            Ok((flags, storage.registry_version(&k)?))
        })();
        let _ = storage.registry_close(k);
        result
    }
    /// Re-read `key`'s flags when its registry version is not `known`:
    /// Ok(None) when the entry was deleted, Ok(Some(None)) when unchanged.
    pub fn refresh(
        storage: &mut Storage,
        key: &str,
        known: u64,
    ) -> Result<Option<Option<(u32, u64)>>> {
        let k = match storage.registry_open(
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &path_units(key),
            READ,
            false,
        ) {
            Ok(k) => k,
            Err(Error::Service(Code::NotFound | Code::DeletePending)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let result = (|| {
            let version = storage.registry_version(&k)?;
            if version == known {
                return Ok(Some(None));
            }
            let flags = flags_of(storage.registry_query(&k, &units(sender::FLAGS)))?
                .unwrap_or(sender::DEFAULT);
            Ok(Some(Some((flags, version))))
        })();
        let _ = storage.registry_close(k);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn senders_are_learned_once_and_forgotten_when_deleted() {
        let mut s = Senders::default();
        assert_eq!(s.flags("App.1"), None);
        s.request("App.1", "File Explorer");
        s.request("App.1", "File Explorer");
        assert_eq!(s.take_pending().len(), 1, "created once");
        assert!(
            !s.learn("App.1", sender::DEFAULT, 3),
            "default: nothing new"
        );
        s.request("App.1", "File Explorer");
        assert!(s.take_pending().is_empty(), "known senders need no entry");
        assert!(s.learn("App.1", sender::ON, 4));
        assert_eq!((s.flags("App.1"), s.version("App.1")), (Some(1), Some(4)));
        assert!(!s.learn("App.1", sender::ON, 5));
        // Invalid stored flags read as the default.
        assert!(s.learn("App.1", 0xff, 6));
        assert_eq!(s.flags("App.1"), Some(sender::DEFAULT));
        assert!(s.forget("App.1") && s.flags("App.1").is_none());
        assert!(!s.forget("App.1"));
        for i in 0..MAX_SENDERS + 3 {
            s.learn(&alloc::format!("App.{i}"), sender::DEFAULT, 1);
        }
        assert_eq!(s.known.len(), MAX_SENDERS);
    }
    #[test]
    fn registry_values_fit_their_limits() {
        let path = path_units("Exe.wingui-editor.exe.0012abcd");
        assert_eq!(
            String::from_utf16(&path).unwrap(),
            "Software\\Doors\\Notifications\\Senders\\Exe.wingui-editor.exe.0012abcd"
        );
        assert!(path.len() <= 96, "registry_open path limit");
        let short = name_value("File Explorer");
        assert_eq!(short.len(), ("File Explorer".len() + 1) * 2);
        assert!(short.ends_with(&[0, 0]));
        let long = name_value(&"n".repeat(80));
        assert_eq!(long.len(), 64);
        // A surrogate pair at the limit is dropped whole.
        let emoji = alloc::format!("{}\u{1F600}", "a".repeat(30));
        let v = name_value(&emoji);
        assert_eq!(v.len(), 62);
        assert!(door_service_client::RegistryValue::new(1, &long).is_ok());
    }
}
