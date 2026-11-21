//! Coherent bounded registry browsing and checked edits; no hosted IPC fallback.
extern crate alloc;
use alloc::{string::String, vec::Vec};
use door_service_client::{
    Error, Key, RegistryEntry, RegistryName, RegistryRoot, RegistryValue, RegistryView, Storage,
};
use door_service_protocol::Code;

pub const MAX_ITEMS: u32 = 256;
pub trait Backend {
    type Key;
    fn open(&mut self, location: &Location, write: bool) -> Result<Self::Key, Error>;
    fn version(&mut self, key: &Self::Key) -> Result<u64, Error>;
    fn child(&mut self, key: &Self::Key, index: u32, version: u64) -> Result<RegistryName, Error>;
    fn value(&mut self, key: &Self::Key, index: u32, version: u64) -> Result<RegistryEntry, Error>;
    fn set(
        &mut self,
        key: &Self::Key,
        name: &[u16],
        value: RegistryValue,
        version: u64,
    ) -> Result<(), Error>;
    fn flush(&mut self) -> Result<(), Error>;
    fn close(&mut self, key: Self::Key) -> Result<(), Error>;
    /// Registry Editor > Delete (value) and Delete key. Keys for deletion
    /// are opened with `open_delete`.
    fn delete_value(&mut self, key: &Self::Key, name: &[u16]) -> Result<(), Error>;
    fn open_delete(&mut self, location: &Location) -> Result<Self::Key, Error>;
    fn delete_key(&mut self, key: &Self::Key) -> Result<(), Error>;
}
impl Backend for Storage {
    type Key = Key;
    fn open(&mut self, l: &Location, write: bool) -> Result<Key, Error> {
        self.registry_open(l.root, l.view, &l.path, if write { 3 } else { 1 }, false)
    }
    fn version(&mut self, k: &Key) -> Result<u64, Error> {
        self.registry_version(k)
    }
    fn child(&mut self, k: &Key, i: u32, v: u64) -> Result<RegistryName, Error> {
        self.registry_enumerate_key(k, i, v)
    }
    fn value(&mut self, k: &Key, i: u32, v: u64) -> Result<RegistryEntry, Error> {
        self.registry_enumerate_value(k, i, v)
    }
    fn set(&mut self, k: &Key, n: &[u16], v: RegistryValue, version: u64) -> Result<(), Error> {
        self.registry_set_checked(k, n, v, version)
    }
    fn flush(&mut self) -> Result<(), Error> {
        Storage::flush(self)
    }
    fn close(&mut self, k: Key) -> Result<(), Error> {
        self.registry_close(k)
    }
    fn delete_value(&mut self, k: &Key, n: &[u16]) -> Result<(), Error> {
        self.registry_delete_value(k, n)
    }
    fn open_delete(&mut self, l: &Location) -> Result<Key, Error> {
        self.registry_open(l.root, l.view, &l.path, 1 | 2 | 4, false)
    }
    fn delete_key(&mut self, k: &Key) -> Result<(), Error> {
        self.registry_delete_key(k)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Location {
    pub root: RegistryRoot,
    pub view: RegistryView,
    pub path: Vec<u16>,
}
impl Default for Location {
    fn default() -> Self {
        Self {
            root: RegistryRoot::CurrentUser,
            view: RegistryView::Bits64,
            path: Vec::new(),
        }
    }
}
impl Location {
    pub fn from_ascii(text: &str, view: RegistryView) -> Result<Self, Error> {
        if text.chars().any(char::is_control) {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let (root, path) = text.split_once('\\').unwrap_or((text, ""));
        let root = if root.eq_ignore_ascii_case("HKCU")
            || root.eq_ignore_ascii_case("HKEY_CURRENT_USER")
        {
            RegistryRoot::CurrentUser
        } else if root.eq_ignore_ascii_case("HKLM")
            || root.eq_ignore_ascii_case("HKEY_LOCAL_MACHINE")
        {
            RegistryRoot::LocalMachine
        } else if root.eq_ignore_ascii_case("HKCR")
            || root.eq_ignore_ascii_case("HKEY_CLASSES_ROOT")
        {
            RegistryRoot::ClassesRoot
        } else {
            return Err(Error::Service(Code::InvalidRequest));
        };
        // Any script is typed directly; `%XXXX` names one UTF-16 unit (an
        // unpaired surrogate, '%' itself or a control character).
        let mut units = Vec::new();
        let mut chars = path.chars();
        while let Some(c) = chars.next() {
            let mut buffer = [0u16; 2];
            let encoded: &[u16] = if c == '%' {
                let mut unit = 0u16;
                for _ in 0..4 {
                    unit = unit * 16
                        + chars
                            .next()
                            .and_then(|c| c.to_digit(16))
                            .ok_or(Error::Service(Code::InvalidRequest))?
                            as u16;
                }
                buffer[0] = unit;
                &buffer[..1]
            } else {
                c.encode_utf16(&mut buffer)
            };
            for &unit in encoded {
                if unit == 0 || units.len() == 96 {
                    return Err(Error::Service(Code::InvalidRequest));
                }
                units.push(unit);
            }
        }
        Ok(Self {
            root,
            view,
            path: units,
        })
    }
    pub fn child(&self, name: &RegistryName) -> Result<Self, Error> {
        let mut next = self.clone();
        if !next.path.is_empty() {
            next.path.push(92);
        }
        next.path.extend_from_slice(name.units());
        if next.path.len() > 96 {
            return Err(Error::Service(Code::Resources));
        }
        Ok(next)
    }
    pub fn parent(&self) -> Option<Self> {
        if self.path.is_empty() {
            return None;
        }
        let mut next = self.clone();
        next.path
            .truncate(self.path.iter().rposition(|u| *u == 92).unwrap_or(0));
        Some(next)
    }
    pub fn display(&self) -> String {
        let mut text = String::from(match self.root {
            RegistryRoot::CurrentUser => "HKEY_CURRENT_USER",
            RegistryRoot::LocalMachine => "HKEY_LOCAL_MACHINE",
            RegistryRoot::ClassesRoot => "HKEY_CLASSES_ROOT",
        });
        if !self.path.is_empty() {
            text.push('\\');
            use core::fmt::Write;
            for item in core::char::decode_utf16(self.path.iter().copied()) {
                match item {
                    Ok(c) if c != '%' && !c.is_control() => text.push(c),
                    Ok(c) => {
                        for unit in c.encode_utf16(&mut [0; 2]) {
                            let _ = write!(text, "%{unit:04X}");
                        }
                    }
                    Err(e) => {
                        let _ = write!(text, "%{:04X}", e.unpaired_surrogate());
                    }
                }
            }
        }
        text
    }
}
pub fn display_units(units: &[u16]) -> String {
    use core::fmt::Write;
    let mut out = String::new();
    // Every script displays as itself; control characters and unpaired
    // surrogates stay visible as \uXXXX.
    for item in core::char::decode_utf16(units.iter().copied()) {
        match item {
            Ok(c) if !c.is_control() => out.push(c),
            Ok(c) => {
                for u in c.encode_utf16(&mut [0; 2]) {
                    let _ = write!(out, "\\u{u:04X}");
                }
            }
            Err(e) => {
                let _ = write!(out, "\\u{:04X}", e.unpaired_surrogate());
            }
        }
    }
    out
}
pub fn same_name(a: &[u16], b: &[u16]) -> bool {
    fn fold(u: u16) -> u16 {
        if (65..=90).contains(&u) { u + 32 } else { u }
    }
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| fold(*a) == fold(*b))
}
/// A service's DisplayName from HKLM\SYSTEM\CurrentControlSet\Services.
pub fn service_display<B: Backend>(backend: &mut B, name: &str) -> Option<String> {
    let path = alloc::format!("HKLM\\SYSTEM\\CurrentControlSet\\Services\\{name}");
    let location = Location::from_ascii(&path, RegistryView::Bits64).ok()?;
    let snapshot = load(backend, &location).ok()?;
    let display: Vec<u16> = "DisplayName".encode_utf16().collect();
    let entry = snapshot
        .values
        .iter()
        .find(|v| same_name(v.name.units(), &display))?;
    if !matches!(entry.value.kind, 1 | 2) {
        return None;
    }
    let units: Vec<u16> = entry
        .value
        .data()
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    Some(String::from_utf16_lossy(&units)).filter(|s| !s.is_empty())
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub location: Location,
    pub version: u64,
    pub children: Vec<RegistryName>,
    pub values: Vec<RegistryEntry>,
}
fn finish<T>(result: Result<T, Error>, close: Result<(), Error>) -> Result<T, Error> {
    match (result, close) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}
pub fn load<B: Backend>(backend: &mut B, location: &Location) -> Result<Snapshot, Error> {
    let key = backend.open(location, false)?;
    let result = (|| {
        let version = backend.version(&key)?;
        let mut children = Vec::new();
        let mut values = Vec::new();
        for i in 0..=MAX_ITEMS {
            match backend.child(&key, i, version) {
                Ok(name) if i < MAX_ITEMS => {
                    if name.units().is_empty()
                        || children
                            .iter()
                            .any(|old: &RegistryName| same_name(old.units(), name.units()))
                    {
                        return Err(Error::Malformed);
                    }
                    children.push(name);
                }
                Ok(_) => return Err(Error::Service(Code::Resources)),
                Err(Error::Service(Code::NotFound)) => break,
                Err(e) => return Err(e),
            }
        }
        for i in 0..=MAX_ITEMS {
            match backend.value(&key, i, version) {
                Ok(entry) if i < MAX_ITEMS => {
                    if RegistryValue::new(entry.value.kind, entry.value.data()).is_err() {
                        return Err(Error::Malformed);
                    }
                    if values
                        .iter()
                        .any(|old: &RegistryEntry| same_name(old.name.units(), entry.name.units()))
                    {
                        return Err(Error::Malformed);
                    }
                    values.push(entry);
                }
                Ok(_) => return Err(Error::Service(Code::Resources)),
                Err(Error::Service(Code::NotFound)) => break,
                Err(e) => return Err(e),
            }
        }
        if backend.version(&key)? != version {
            return Err(Error::Service(Code::Changed));
        }
        Ok(Snapshot {
            location: location.clone(),
            version,
            children,
            values,
        })
    })();
    let closed = backend.close(key);
    finish(result, closed)
}
/// The service checks the snapshot and sets the value in one serialized request.
/// A failed return must not be treated as a confirmed save or automatically retried.
pub fn save<B: Backend>(
    backend: &mut B,
    snapshot: &Snapshot,
    name: &[u16],
    value: RegistryValue,
    new: bool,
) -> Result<(), Error> {
    RegistryValue::new(value.kind, value.data())?;
    let exists = snapshot
        .values
        .iter()
        .any(|v| same_name(v.name.units(), name));
    if (new && exists) || (!new && !exists) {
        return Err(Error::Service(if new {
            Code::Exists
        } else {
            Code::NotFound
        }));
    }
    let key = backend.open(&snapshot.location, true)?;
    let result = backend
        .set(&key, name, value, snapshot.version)
        .and_then(|_| backend.flush());
    let closed = backend.close(key);
    finish(result, closed)
}

/// Delete value `name` of the key the snapshot was read from (checked
/// against its version first, as saves are).
pub fn delete_value<B: Backend>(
    backend: &mut B,
    snapshot: &Snapshot,
    name: &[u16],
) -> Result<(), Error> {
    if !snapshot
        .values
        .iter()
        .any(|v| same_name(v.name.units(), name))
    {
        return Err(Error::Service(Code::NotFound));
    }
    let key = backend.open(&snapshot.location, true)?;
    let result = (|| {
        if backend.version(&key)? != snapshot.version {
            return Err(Error::Service(Code::Changed));
        }
        backend.delete_value(&key, name)?;
        backend.flush()
    })();
    let closed = backend.close(key);
    finish(result, closed)
}
/// Rename value `old` to `new`: the data moves under the new name, then the
/// old name goes (a name already in use is refused before anything moves).
pub fn rename_value<B: Backend>(
    backend: &mut B,
    snapshot: &Snapshot,
    old: &[u16],
    new: &[u16],
    value: RegistryValue,
) -> Result<(), Error> {
    if same_name(old, new) {
        return save(backend, snapshot, new, value, false);
    }
    if snapshot
        .values
        .iter()
        .any(|v| same_name(v.name.units(), new))
    {
        return Err(Error::Service(Code::Exists));
    }
    if !snapshot
        .values
        .iter()
        .any(|v| same_name(v.name.units(), old))
    {
        return Err(Error::Service(Code::NotFound));
    }
    let key = backend.open(&snapshot.location, true)?;
    let result = (|| {
        backend.set(&key, new, value, snapshot.version)?;
        backend.delete_value(&key, old)?;
        backend.flush()
    })();
    let closed = backend.close(key);
    finish(result, closed)
}
/// Delete the key at `location` (it must have no subkeys). The root keys
/// themselves can't be deleted.
pub fn delete_key<B: Backend>(backend: &mut B, location: &Location) -> Result<(), Error> {
    if location.path.is_empty() {
        return Err(Error::Service(Code::Denied));
    }
    let key = backend.open_delete(location)?;
    let result = backend.delete_key(&key).and_then(|()| backend.flush());
    let closed = backend.close(key);
    finish(result, closed)
}
/// Registry Editor > Export: the key and its subkeys (at most `depth`
/// levels) in the Windows `.reg` format (version 5.00, CRLF lines).
pub fn export<B: Backend>(
    backend: &mut B,
    location: &Location,
    depth: u32,
) -> Result<String, Error> {
    let mut out = String::from("Windows Registry Editor Version 5.00\r\n");
    export_key(backend, location, depth, &mut out)?;
    Ok(out)
}
fn export_key<B: Backend>(
    backend: &mut B,
    location: &Location,
    depth: u32,
    out: &mut String,
) -> Result<(), Error> {
    let snapshot = load(backend, location)?;
    out.push_str("\r\n[");
    out.push_str(&location.display());
    out.push_str("]\r\n");
    for entry in &snapshot.values {
        let name = display_units(entry.name.units());
        if name.is_empty() {
            out.push('@');
        } else {
            out.push('"');
            out.push_str(&reg_escape(&name));
            out.push('"');
        }
        out.push('=');
        let data = entry.value.data();
        match entry.value.kind {
            1 => {
                let units: Vec<u16> = data
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .take_while(|u| *u != 0)
                    .collect();
                out.push('"');
                out.push_str(&reg_escape(&String::from_utf16_lossy(&units)));
                out.push('"');
            }
            4 if data.len() == 4 => {
                let v = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                out.push_str(&alloc::format!("dword:{v:08x}"));
            }
            kind => {
                out.push_str(&match kind {
                    3 => String::from("hex:"),
                    k => alloc::format!("hex({k:x}):"),
                });
                let hex: Vec<String> = data.iter().map(|b| alloc::format!("{b:02x}")).collect();
                out.push_str(&hex.join(","));
            }
        }
        out.push_str("\r\n");
    }
    if depth > 0 {
        for child in &snapshot.children {
            export_key(backend, &location.child(child)?, depth - 1, out)?;
        }
    }
    Ok(())
}
fn reg_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    #[test]
    fn address_roundtrips_unicode_surrogates_percent_and_escape_like_names() {
        for path in [
            vec![0x4e2d, 92, 0xd800],
            vec![37, 52, 69, 50, 68],
            vec![117, 68, 56, 48, 48],
            vec![65; 96],
        ] {
            let location = Location {
                root: RegistryRoot::CurrentUser,
                view: RegistryView::Bits32,
                path,
            };
            assert_eq!(
                Location::from_ascii(&location.display(), location.view).unwrap(),
                location
            );
        }
        for path in ["HKCU\\%", "HKCU\\%XYZW", "HKCU\\%0000", "HKCU\\%D80"] {
            assert!(Location::from_ascii(path, RegistryView::Bits64).is_err());
        }
        assert_ne!(
            Location::from_ascii("HKCU\\%D800", RegistryView::Bits64).unwrap(),
            Location::from_ascii("HKCU\\uD800", RegistryView::Bits64).unwrap()
        );
    }
    fn name(s: &[u16]) -> RegistryName {
        RegistryName::new(s).unwrap()
    }
    struct Memory {
        children: Vec<RegistryName>,
        values: Vec<RegistryEntry>,
        version: u64,
        opened: usize,
        closed: usize,
        sets: usize,
        flushes: usize,
        versions: usize,
        late_change: bool,
        flush_error: bool,
    }
    impl Default for Memory {
        fn default() -> Self {
            Self {
                children: vec![name(&[65, 0xd800])],
                values: vec![RegistryEntry {
                    name: name(&[]),
                    value: RegistryValue::dword(7),
                }],
                version: 5,
                opened: 0,
                closed: 0,
                sets: 0,
                flushes: 0,
                versions: 0,
                late_change: false,
                flush_error: false,
            }
        }
    }
    impl Backend for Memory {
        type Key = usize;
        fn open(&mut self, _: &Location, _: bool) -> Result<usize, Error> {
            self.opened += 1;
            Ok(self.opened)
        }
        fn version(&mut self, _: &usize) -> Result<u64, Error> {
            self.versions += 1;
            Ok(self.version + u64::from(self.late_change && self.versions > 1))
        }
        fn child(&mut self, _: &usize, i: u32, v: u64) -> Result<RegistryName, Error> {
            assert_eq!(v, self.version);
            self.children
                .get(i as usize)
                .copied()
                .ok_or(Error::Service(Code::NotFound))
        }
        fn value(&mut self, _: &usize, i: u32, v: u64) -> Result<RegistryEntry, Error> {
            assert_eq!(v, self.version);
            self.values
                .get(i as usize)
                .copied()
                .ok_or(Error::Service(Code::NotFound))
        }
        fn set(
            &mut self,
            _: &usize,
            n: &[u16],
            v: RegistryValue,
            observed: u64,
        ) -> Result<(), Error> {
            self.sets += 1;
            if observed != self.version {
                return Err(Error::Service(Code::Changed));
            }
            if let Some(entry) = self
                .values
                .iter_mut()
                .find(|v| same_name(v.name.units(), n))
            {
                entry.value = v;
            } else {
                self.values.push(RegistryEntry {
                    name: name(n),
                    value: v,
                });
            }
            self.version += 1;
            Ok(())
        }
        fn flush(&mut self) -> Result<(), Error> {
            self.flushes += 1;
            if self.flush_error {
                Err(Error::Service(Code::Io))
            } else {
                Ok(())
            }
        }
        fn close(&mut self, _: usize) -> Result<(), Error> {
            self.closed += 1;
            Ok(())
        }
        fn delete_value(&mut self, _: &usize, n: &[u16]) -> Result<(), Error> {
            let before = self.values.len();
            self.values.retain(|v| !same_name(v.name.units(), n));
            if self.values.len() == before {
                return Err(Error::Service(Code::NotFound));
            }
            self.version += 1;
            Ok(())
        }
        fn open_delete(&mut self, l: &Location) -> Result<usize, Error> {
            self.open(l, true)
        }
        fn delete_key(&mut self, _: &usize) -> Result<(), Error> {
            if self.children.is_empty() {
                Ok(())
            } else {
                Err(Error::Service(Code::Denied))
            }
        }
    }
    #[test]
    fn values_delete_and_rename_by_version_and_export_as_reg_text() {
        let mut backend = Memory::default();
        let location = Location::default();
        let s = load(&mut backend, &location).unwrap();
        // Rename the default value to "Count", then delete it.
        rename_value(&mut backend, &s, &[], &[67, 111], RegistryValue::dword(7)).unwrap();
        assert_eq!(backend.values.len(), 1);
        assert_eq!(backend.values[0].name.units(), &[67, 111]);
        let s = load(&mut backend, &location).unwrap();
        assert_eq!(
            delete_value(&mut backend, &s, &[90]),
            Err(Error::Service(Code::NotFound))
        );
        delete_value(&mut backend, &s, &[67, 111]).unwrap();
        assert!(backend.values.is_empty());
        // A stale snapshot is refused.
        assert_eq!(
            rename_value(&mut backend, &s, &[67, 111], &[68], RegistryValue::dword(1)),
            Err(Error::Service(Code::Changed))
        );
        // Root keys and keys with subkeys stay.
        assert_eq!(
            delete_key(&mut backend, &location),
            Err(Error::Service(Code::Denied))
        );
        let child = location.child(&s.children[0]).unwrap();
        assert!(delete_key(&mut backend, &child).is_err(), "it has a subkey");
        backend.values.push(RegistryEntry {
            name: name(&[78]),
            value: RegistryValue::dword(0x1f),
        });
        let text = export(&mut backend, &location, 0).unwrap();
        assert!(text.starts_with("Windows Registry Editor Version 5.00\r\n\r\n["));
        assert!(text.contains("\"N\"=dword:0000001f\r\n"), "{text}");
    }
    #[test]
    fn coherent_load_preserves_identity_and_every_failure_closes_its_handle() {
        let mut backend = Memory::default();
        let location = Location::default();
        let s = load(&mut backend, &location).unwrap();
        assert_eq!(s.version, 5);
        assert_eq!(s.children[0].units(), &[65, 0xd800]);
        assert_eq!(backend.closed, 1);
        let child = location.child(&s.children[0]).unwrap();
        assert_eq!(child.path, &[65, 0xd800]);
        assert_eq!(child.parent(), Some(location.clone()));
        let mut changed = Memory {
            late_change: true,
            ..Memory::default()
        };
        assert_eq!(
            load(&mut changed, &location).unwrap_err(),
            Error::Service(Code::Changed)
        );
        assert_eq!(changed.closed, 1);
        let mut duplicate = Memory::default();
        duplicate.children.push(name(&[97, 0xd800]));
        assert_eq!(
            load(&mut duplicate, &location).unwrap_err(),
            Error::Malformed
        );
        assert_eq!(duplicate.closed, 1);
        let mut bad = Memory::default();
        bad.values[0].value.kind = 11;
        assert_eq!(load(&mut bad, &location).unwrap_err(), Error::Malformed);
        assert_eq!(bad.closed, 1);
        let mut full = Memory {
            children: (0..=MAX_ITEMS)
                .map(|i| name(&[65, (0x100 + i) as u16]))
                .collect(),
            ..Memory::default()
        };
        assert_eq!(
            load(&mut full, &location).unwrap_err(),
            Error::Service(Code::Resources)
        );
        assert_eq!(full.closed, 1);
    }
    #[test]
    fn checked_edits_cannot_overwrite_newer_data_and_save_requires_flush() {
        let mut b = Memory::default();
        let s = load(&mut b, &Location::default()).unwrap();
        assert_eq!(
            save(&mut b, &s, &[], RegistryValue::dword(9), true),
            Err(Error::Service(Code::Exists))
        );
        assert_eq!(b.sets, 0);
        save(&mut b, &s, &[], RegistryValue::dword(9), false).unwrap();
        assert_eq!(b.values[0].value, RegistryValue::dword(9));
        assert_eq!((b.sets, b.flushes, b.closed), (1, 1, 2));
        assert_eq!(
            save(&mut b, &s, &[], RegistryValue::dword(11), false),
            Err(Error::Service(Code::Changed))
        );
        assert_eq!(b.values[0].value, RegistryValue::dword(9));
        assert_eq!(b.flushes, 1);
        assert_eq!(b.closed, 3);
        let fresh = load(&mut b, &Location::default()).unwrap();
        b.flush_error = true;
        assert_eq!(
            save(&mut b, &fresh, &[65], RegistryValue::dword(3), true),
            Err(Error::Service(Code::Io))
        );
        assert_eq!(b.closed, b.opened);
        assert!(!fresh.values.iter().any(|v| v.name.units() == [65]));
    }
    #[test]
    fn roots_views_and_paths_are_explicit_and_bounded() {
        let location = Location::from_ascii("hKcU\\Software\\Doors", RegistryView::Bits32).unwrap();
        assert_eq!(location.display(), "HKEY_CURRENT_USER\\Software\\Doors");
        assert_eq!(location.view, RegistryView::Bits32);
        assert_eq!(
            location.parent().unwrap().display(),
            "HKEY_CURRENT_USER\\Software"
        );
        assert_eq!(Location::default().parent(), None);
        for bad in ["HKU\\OtherUser", "HKEY_CURRENT_USER\n", "HKCU\\a\u{7}"] {
            assert!(Location::from_ascii(bad, RegistryView::Bits64).is_err());
        }
        // Any script is typed and shown as itself (UTF-16 units underneath).
        let unicode = Location::from_ascii("HKCU\\é\\日本\\😀", RegistryView::Bits64).unwrap();
        assert_eq!(unicode.path, [0xe9, 92, 0x65e5, 0x672c, 92, 0xd83d, 0xde00]);
        assert_eq!(unicode.display(), "HKEY_CURRENT_USER\\é\\日本\\😀");
        assert_eq!(display_units(&[0xd800, 0x41]), "\\uD800A");
        assert!(
            Location::from_ascii(
                &alloc::format!("HKCU\\{}", "A".repeat(97)),
                RegistryView::Bits64
            )
            .is_err()
        );
    }
}
