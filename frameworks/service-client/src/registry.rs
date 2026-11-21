use super::{Error, Result, Storage};
use door_abi::Handle;
use door_service_protocol::{Code, REGISTRY_VALUE_BYTES, opcode};
#[derive(Debug)]
pub struct Key {
    token: u64,
    owner: (Handle, u64),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RegistryRoot {
    LocalMachine = 0,
    CurrentUser = 1,
    ClassesRoot = 2,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RegistryView {
    Bits32 = 32,
    Bits64 = 64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegistryValue {
    pub kind: u8,
    bytes: [u8; REGISTRY_VALUE_BYTES],
    length: usize,
}
/// Lossless counted UTF-16. Display conversion never changes registry identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegistryName {
    units: [u16; 48],
    length: usize,
}
impl RegistryName {
    pub fn new(units: &[u16]) -> Result<Self> {
        if units.len() > 48 || units.contains(&0) {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut name = Self {
            units: [0; 48],
            length: units.len(),
        };
        name.units[..units.len()].copy_from_slice(units);
        Ok(name)
    }
    pub fn units(&self) -> &[u16] {
        &self.units[..self.length]
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegistryEntry {
    pub name: RegistryName,
    pub value: RegistryValue,
}
fn enumerated(data: &[u8], value: bool) -> Result<(RegistryName, Option<RegistryValue>)> {
    // [name units][type][bytes here][0][whole length u16][0; 2]: a longer
    // value's rest is read with REGISTRY_QUERY.
    if data.len() < 8
        || data[0] > 48
        || data[3] != 0
        || data[6..8] != [0; 2]
        || (!value && (data[0] == 0 || data[1..6] != [0; 5]))
    {
        return Err(Error::Malformed);
    }
    let length = data[0] as usize;
    let end = 8 + length * 2;
    if data.len() != end + data[2] as usize {
        return Err(Error::Malformed);
    }
    let mut name = RegistryName {
        units: [0; 48],
        length,
    };
    for (out, bytes) in name.units.iter_mut().zip(data[8..end].chunks_exact(2)) {
        *out = u16::from_le_bytes(bytes.try_into().unwrap());
        if *out == 0 {
            return Err(Error::Malformed);
        }
    }
    let whole = usize::from(u16::from_le_bytes([data[4], data[5]]));
    if value && (data[2] > 80 || whole < usize::from(data[2]) || whole > REGISTRY_VALUE_BYTES) {
        return Err(Error::Malformed);
    }
    // A value longer than this reply: None, the caller queries it.
    let entry = if value && whole <= usize::from(data[2]) {
        Some(RegistryValue::new(data[1], &data[end..]).map_err(|_| Error::Malformed)?)
    } else {
        None
    };
    Ok((name, entry))
}
impl RegistryValue {
    pub fn new(kind: u8, bytes: &[u8]) -> Result<Self> {
        if bytes.len() > REGISTRY_VALUE_BYTES || ![0, 1, 2, 3, 4, 7, 11].contains(&kind) {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let valid = match kind {
            4 => bytes.len() == 4,
            11 => bytes.len() == 8,
            1 | 2 => bytes.len().is_multiple_of(2) && bytes.ends_with(&[0, 0]),
            7 => bytes.len().is_multiple_of(2) && bytes.ends_with(&[0, 0, 0, 0]),
            _ => true,
        };
        if !valid {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut value = Self {
            kind,
            bytes: [0; REGISTRY_VALUE_BYTES],
            length: bytes.len(),
        };
        value.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(value)
    }
    pub fn data(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
    pub fn dword(value: u32) -> Self {
        Self::new(4, &value.to_le_bytes()).unwrap()
    }
}
fn put_units(input: &[u16], output: &mut [u8]) {
    for (unit, bytes) in input.iter().zip(output.chunks_exact_mut(2)) {
        bytes.copy_from_slice(&unit.to_le_bytes());
    }
}
impl Storage {
    pub fn registry_version(&mut self, key: &Key) -> Result<u64> {
        self.operation(|storage| {
            storage.check_key(key)?;
            storage
                .connection
                .call(opcode::REGISTRY_VERSION, &key.token.to_le_bytes(), &[])?
                .scalar()
        })
    }
    fn registry_enumerate(
        &mut self,
        key: &Key,
        index: u32,
        observed: u64,
        values: bool,
    ) -> Result<(RegistryName, Option<RegistryValue>)> {
        self.operation(|storage| {
            storage.check_key(key)?;
            let mut body = [0; 24];
            body[..8].copy_from_slice(&key.token.to_le_bytes());
            body[8..16].copy_from_slice(&observed.to_le_bytes());
            body[16..20].copy_from_slice(&index.to_le_bytes());
            let reply = storage.connection.call(
                if values {
                    opcode::REGISTRY_ENUM_VALUE
                } else {
                    opcode::REGISTRY_ENUM_KEY
                },
                &body,
                &[],
            )?;
            enumerated(reply.data()?, values)
        })
    }
    pub fn registry_enumerate_key(
        &mut self,
        key: &Key,
        index: u32,
        observed: u64,
    ) -> Result<RegistryName> {
        self.registry_enumerate(key, index, observed, false)
            .map(|e| e.0)
    }
    pub fn registry_enumerate_value(
        &mut self,
        key: &Key,
        index: u32,
        observed: u64,
    ) -> Result<RegistryEntry> {
        let (name, value) = self.registry_enumerate(key, index, observed, true)?;
        let value = match value {
            Some(value) => value,
            None => self.registry_query(key, name.units())?,
        };
        Ok(RegistryEntry { name, value })
    }
    pub fn registry_open(
        &mut self,
        root: RegistryRoot,
        view: RegistryView,
        path: &[u16],
        access: u16,
        create: bool,
    ) -> Result<Key> {
        self.operation(|storage| {
            use door_service_protocol::REGISTRY_PATH_UNITS;
            if path.len() > REGISTRY_PATH_UNITS {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0u8; 8 + 2 * REGISTRY_PATH_UNITS];
            body[0] = root as u8;
            body[1] = view as u8;
            body[2] = create as u8;
            body[4..6].copy_from_slice(&access.to_le_bytes());
            body[6..8].copy_from_slice(&(path.len() as u16).to_le_bytes());
            put_units(path, &mut body[8..8 + path.len() * 2]);
            // A key path past one packet goes as a long request.
            let token = storage
                .long_call(opcode::REGISTRY_OPEN, &body[..8 + path.len() * 2])?
                .token()?;
            Ok(Key {
                token,
                owner: storage.connection.key(),
            })
        })
    }
    fn check_key(&self, key: &Key) -> Result<()> {
        if key.owner != self.connection.key() {
            return Err(Error::Service(Code::InvalidHandle));
        }
        Ok(())
    }
    /// The whole value, read in as many requests as its length needs.
    pub fn registry_query(&mut self, key: &Key, name: &[u16]) -> Result<RegistryValue> {
        self.operation(|storage| {
            storage.check_key(key)?;
            if name.len() > 48 {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut bytes = [0u8; REGISTRY_VALUE_BYTES];
            let mut kind = 0;
            let mut total = None;
            let mut read = 0;
            loop {
                let mut body = [0; 112];
                body[..8].copy_from_slice(&key.token.to_le_bytes());
                body[8..10].copy_from_slice(&(name.len() as u16).to_le_bytes());
                body[12..16].copy_from_slice(&(read as u32).to_le_bytes());
                put_units(name, &mut body[16..16 + name.len() * 2]);
                let reply = storage.connection.call(
                    opcode::REGISTRY_QUERY,
                    &body[..16 + name.len() * 2],
                    &[],
                )?;
                let data = reply.data()?;
                if data.len() < 8 || data[1] != 0 || data[6..8] != [0; 2] {
                    return Err(Error::Malformed);
                }
                let whole = usize::from(u16::from_le_bytes([data[2], data[3]]));
                let here = usize::from(u16::from_le_bytes([data[4], data[5]]));
                // The value changed between requests: start again.
                if total.is_some_and(|t| t != whole) || (read > 0 && kind != data[0]) {
                    read = 0;
                    total = None;
                    continue;
                }
                if data.len() != 8 + here || whole > REGISTRY_VALUE_BYTES || read + here > whole {
                    return Err(Error::Malformed);
                }
                kind = data[0];
                total = Some(whole);
                bytes[read..read + here].copy_from_slice(&data[8..]);
                read += here;
                if read == whole {
                    return RegistryValue::new(kind, &bytes[..whole]).map_err(|_| Error::Malformed);
                }
                if here == 0 {
                    return Err(Error::Malformed);
                }
            }
        })
    }
    pub fn registry_set(&mut self, key: &Key, name: &[u16], value: RegistryValue) -> Result<()> {
        self.registry_set_inner(key, name, value, None)
    }
    /// Delete value `name` of an open key (the key needs write access).
    pub fn registry_delete_value(&mut self, key: &Key, name: &[u16]) -> Result<()> {
        self.operation(|storage| {
            storage.check_key(key)?;
            if name.len() > 48 {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0; 112];
            body[..8].copy_from_slice(&key.token.to_le_bytes());
            body[8..10].copy_from_slice(&(name.len() as u16).to_le_bytes());
            put_units(name, &mut body[16..16 + name.len() * 2]);
            storage
                .connection
                .call(
                    opcode::REGISTRY_DELETE_VALUE,
                    &body[..16 + name.len() * 2],
                    &[],
                )?
                .empty()
        })
    }
    /// Delete an open key with no subkeys (the key needs delete access);
    /// the handle stays open until closed.
    pub fn registry_delete_key(&mut self, key: &Key) -> Result<()> {
        self.operation(|storage| {
            storage.check_key(key)?;
            storage
                .connection
                .call(opcode::REGISTRY_DELETE_KEY, &key.token.to_le_bytes(), &[])?
                .empty()
        })
    }
    /// Check the snapshot version and mutate within the same service request.
    pub fn registry_set_checked(
        &mut self,
        key: &Key,
        name: &[u16],
        value: RegistryValue,
        observed: u64,
    ) -> Result<()> {
        self.registry_set_inner(key, name, value, Some(observed))
    }
    fn registry_set_inner(
        &mut self,
        key: &Key,
        name: &[u16],
        value: RegistryValue,
        observed: Option<u64>,
    ) -> Result<()> {
        self.operation(|storage| {
            storage.check_key(key)?;
            if name.len() > 48 {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0; door_service_protocol::BODY_BYTES];
            body[..8].copy_from_slice(&key.token.to_le_bytes());
            body[8] = value.kind;
            body[10..12].copy_from_slice(&(name.len() as u16).to_le_bytes());
            let header = if let Some(version) = observed {
                body[16..24].copy_from_slice(&version.to_le_bytes());
                24
            } else {
                16
            };
            let end = header + name.len() * 2;
            put_units(name, &mut body[header..end]);
            // Inline when it fits the request (and a byte count), else
            // staged in parts first.
            let inline = value.length <= 255 && end + value.length <= body.len();
            if inline {
                body[9] = value.length as u8;
                body[end..end + value.length].copy_from_slice(value.data());
            } else {
                body[12..14].copy_from_slice(&(value.length as u16).to_le_bytes());
                let mut offset = 0;
                for part in value.data().chunks(door_service_protocol::BODY_BYTES - 16) {
                    let mut stage = [0; door_service_protocol::BODY_BYTES];
                    stage[..8].copy_from_slice(&key.token.to_le_bytes());
                    stage[8..10].copy_from_slice(&(offset as u16).to_le_bytes());
                    stage[10..12].copy_from_slice(&(value.length as u16).to_le_bytes());
                    stage[16..16 + part.len()].copy_from_slice(part);
                    storage
                        .connection
                        .call(opcode::REGISTRY_STAGE_VALUE, &stage[..16 + part.len()], &[])?
                        .empty()?;
                    offset += part.len();
                }
            }
            let length = end + if inline { value.length } else { 0 };
            storage
                .connection
                .call(
                    if observed.is_some() {
                        opcode::REGISTRY_SET_CHECKED
                    } else {
                        opcode::REGISTRY_SET
                    },
                    &body[..length],
                    &[],
                )?
                .empty()
        })
    }
    /// The machine's Trusted Root store, as Windows keeps it: each entry's
    /// serialized certificate properties
    /// (`HKLM\SOFTWARE\Microsoft\SystemCertificates\ROOT\Certificates\<SHA-1>`,
    /// value `Blob`) handed to `each`, at most `limit` entries. An absent
    /// store has none.
    pub fn root_store_blobs(&mut self, limit: u32, mut each: impl FnMut(&[u8])) -> Result<()> {
        const PATH: &str = "SOFTWARE\\Microsoft\\SystemCertificates\\ROOT\\Certificates";
        let mut path = [0u16; PATH.len()];
        for (unit, byte) in path.iter_mut().zip(PATH.bytes()) {
            *unit = u16::from(byte);
        }
        let store = match self.registry_open(
            RegistryRoot::LocalMachine,
            RegistryView::Bits64,
            &path,
            1,
            false,
        ) {
            Ok(key) => key,
            Err(Error::Service(Code::NotFound)) => return Ok(()),
            Err(e) => return Err(e),
        };
        let blob = [
            u16::from(b'B'),
            u16::from(b'l'),
            u16::from(b'o'),
            u16::from(b'b'),
        ];
        let result = (|| {
            let version = self.registry_version(&store)?;
            for index in 0..limit {
                let name = match self.registry_enumerate_key(&store, index, version) {
                    Ok(name) => name,
                    Err(Error::Service(Code::NotFound)) => break,
                    Err(e) => return Err(e),
                };
                let mut sub = [0u16; PATH.len() + 1 + 48];
                sub[..PATH.len()].copy_from_slice(&path);
                sub[PATH.len()] = u16::from(b'\\');
                let length = PATH.len() + 1 + name.units().len();
                sub[PATH.len() + 1..length].copy_from_slice(name.units());
                let Ok(entry) = self.registry_open(
                    RegistryRoot::LocalMachine,
                    RegistryView::Bits64,
                    &sub[..length],
                    1,
                    false,
                ) else {
                    continue;
                };
                if let Ok(value) = self.registry_query(&entry, &blob)
                    && value.kind == 3
                {
                    each(value.data());
                }
                let _ = self.registry_close(entry);
            }
            Ok(())
        })();
        let _ = self.registry_close(store);
        result
    }
    pub fn registry_close(&mut self, key: Key) -> Result<()> {
        self.operation(|storage| {
            storage.check_key(&key)?;
            storage
                .connection
                .call(opcode::REGISTRY_CLOSE, &key.token.to_le_bytes(), &[])?
                .empty()
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enumeration_decoder_preserves_default_and_unpaired_names_and_rejects_malformed_data() {
        let default = [0, 4, 4, 0, 4, 0, 0, 0, 0x78, 0x56, 0x34, 0x12];
        let (name, value) = enumerated(&default, true).unwrap();
        assert!(name.units().is_empty());
        assert_eq!(value.unwrap(), RegistryValue::dword(0x12345678));
        assert_eq!(enumerated(&default, false), Err(Error::Malformed));
        let name = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0xd8, 65, 0];
        assert_eq!(enumerated(&name, false).unwrap().0.units(), &[0xd800, 65]);
        for length in 0..name.len() {
            assert_eq!(enumerated(&name[..length], false), Err(Error::Malformed));
        }
        for (index, value) in [(0, 49), (1, 1), (2, 1), (3, 1), (7, 1), (9, 0)] {
            let mut bad = name;
            bad[index] = value;
            assert_eq!(enumerated(&bad, false), Err(Error::Malformed));
        }
        for (index, value) in [(1, 5), (2, 65), (3, 1)] {
            let mut bad = default;
            bad[index] = value;
            assert_eq!(enumerated(&bad, true), Err(Error::Malformed));
        }
        let mut too_long = [0; 177];
        too_long[0] = 48;
        too_long[1] = 3;
        too_long[2] = 73;
        for pair in too_long[8..104].chunks_exact_mut(2) {
            pair[0] = 65;
        }
        assert_eq!(enumerated(&too_long, true), Err(Error::Malformed));
    }
    #[test]
    fn typed_values_validate_terminators_width_and_capacity() {
        assert_eq!(
            RegistryValue::dword(0x12345678).data(),
            [0x78, 0x56, 0x34, 0x12]
        );
        assert!(RegistryValue::new(4, &[1; 3]).is_err());
        assert!(RegistryValue::new(11, &[0; 7]).is_err());
        assert!(RegistryValue::new(1, &[]).is_err());
        assert!(RegistryValue::new(1, &[0; 3]).is_err());
        assert!(RegistryValue::new(7, &[0; 2]).is_err());
        assert!(RegistryValue::new(7, &[0; 4]).is_ok());
        assert!(RegistryValue::new(3, &[0; 2048]).is_ok());
        assert!(RegistryValue::new(3, &[0; 2049]).is_err());
        assert!(RegistryValue::new(255, &[]).is_err());
    }
}
