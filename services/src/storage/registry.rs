use super::*;
use door_storage_core::persistence::{
    KeyHandle, MAX_VALUE_BYTES, RegistryRoot, RegistryType, RegistryView, Value,
};
pub(super) struct OpenKey {
    pub channel: Handle,
    pid: u64,
    session: u64,
    epoch: u64,
    token: u64,
    pub key: KeyHandle,
}
/// A value too long for one request, staged for the open key `token`.
pub(super) struct Stage {
    token: u64,
    total: usize,
    received: usize,
    bytes: [u8; MAX_VALUE_BYTES],
}
impl Stage {
    pub(super) const EMPTY: Self = Self {
        token: 0,
        total: 0,
        received: 0,
        bytes: [0; MAX_VALUE_BYTES],
    };
}
fn units<'a>(bytes: &[u8], output: &'a mut [u16]) -> Result<&'a [u16], Code> {
    if !bytes.len().is_multiple_of(2) || bytes.len() / 2 > output.len() {
        return Err(Code::InvalidRequest);
    }
    let count = bytes.len() / 2;
    for (source, destination) in bytes.chunks_exact(2).zip(output.iter_mut()) {
        *destination = u16::from_le_bytes(source.try_into().unwrap());
    }
    Ok(&output[..count])
}
fn kind(value: u8) -> Result<RegistryType, Code> {
    Ok(match value {
        0 => RegistryType::None,
        1 => RegistryType::String,
        2 => RegistryType::ExpandString,
        3 => RegistryType::Binary,
        4 => RegistryType::Dword,
        7 => RegistryType::MultiString,
        11 => RegistryType::Qword,
        _ => return Err(Code::Unsupported),
    })
}
impl Broker<'_> {
    fn key(&self, channel: Handle, peer: Identity, token: u64) -> Result<usize, Code> {
        self.keys
            .iter()
            .position(|entry| {
                entry.as_ref().is_some_and(|e| {
                    e.channel == channel
                        && e.pid == peer.pid
                        && e.session == peer.session
                        && e.epoch == peer.epoch
                        && e.token == token
                })
            })
            .ok_or(Code::InvalidHandle)
    }
    pub(super) fn registry(
        &mut self,
        channel: Handle,
        peer: Identity,
        subject: Subject,
        operation: u32,
        body: &[u8],
    ) -> Result<Response, Code> {
        match operation {
            opcode::REGISTRY_OPEN => {
                if body.len() < 8
                    || body[2] > 1
                    || body[3] != 0
                    || body.len()
                        != 8 + usize::from(u16::from_le_bytes(body[6..8].try_into().unwrap())) * 2
                {
                    return Err(Code::InvalidRequest);
                }
                let root = match body[0] {
                    0 => RegistryRoot::LocalMachine,
                    1 => RegistryRoot::CurrentUser,
                    2 => RegistryRoot::ClassesRoot,
                    _ => return Err(Code::InvalidRequest),
                };
                let view = match body[1] {
                    32 => RegistryView::Bits32,
                    64 => RegistryView::Bits64,
                    _ => return Err(Code::InvalidRequest),
                };
                let access = Access(u16::from_le_bytes(body[4..6].try_into().unwrap()));
                let mut path = [0u16; door_service_protocol::REGISTRY_PATH_UNITS];
                let path = units(&body[8..], &mut path)?;
                let index = self
                    .keys
                    .iter()
                    .position(Option::is_none)
                    .ok_or(Code::Resources)?;
                let next = self.next.checked_add(1).ok_or(Code::Resources)?;
                let key = self
                    .volume
                    .registry_open(subject, root, view, path, access, body[2] != 0)
                    .map_err(persistence_error)?;
                let token = self.next;
                self.next = next;
                self.keys[index] = Some(OpenKey {
                    channel,
                    pid: peer.pid,
                    session: peer.session,
                    epoch: peer.epoch,
                    token,
                    key,
                });
                Ok(Response::scalar(token))
            }
            // [key u64][name units u16][0 u16][offset u32][name]: reply
            // [type u8][0][whole length u16][bytes here u16][0 u16][bytes].
            opcode::REGISTRY_QUERY => {
                if body.len() < 16
                    || body[10..12] != [0; 2]
                    || body.len()
                        != 16 + usize::from(u16::from_le_bytes(body[8..10].try_into().unwrap())) * 2
                {
                    return Err(Code::InvalidRequest);
                }
                let offset = u32::from_le_bytes(body[12..16].try_into().unwrap()) as usize;
                let index = self.key(channel, peer, u64at(body, 0))?;
                let mut name = [0; 48];
                let name = units(&body[16..], &mut name)?;
                let mut response = Response::empty();
                let room = response.body.len() - 8;
                let (kind, total, copied) = self
                    .volume
                    .registry_read(
                        &self.keys[index].as_ref().unwrap().key,
                        name,
                        offset,
                        &mut response.body[8..8 + room],
                    )
                    .map_err(persistence_error)?;
                if offset > total {
                    return Err(Code::InvalidRequest);
                }
                response.body[0] = kind as u8;
                response.body[2..4].copy_from_slice(&(total as u16).to_le_bytes());
                response.body[4..6].copy_from_slice(&(copied as u16).to_le_bytes());
                response.len = 8 + copied;
                Ok(response)
            }
            opcode::REGISTRY_SET | opcode::REGISTRY_SET_CHECKED => {
                let header = if operation == opcode::REGISTRY_SET_CHECKED {
                    24
                } else {
                    16
                };
                // [key][type u8][inline bytes u8][name units u16]
                // [staged bytes u16][0 u16]: a value too long for the body
                // was staged first (REGISTRY_STAGE_VALUE) for this key.
                if body.len() < header || body[14..16] != [0; 2] {
                    return Err(Code::InvalidRequest);
                }
                let names = usize::from(u16::from_le_bytes(body[10..12].try_into().unwrap())) * 2;
                let inline = usize::from(body[9]);
                let long = usize::from(u16::from_le_bytes(body[12..14].try_into().unwrap()));
                if body.len() != header + names + inline
                    || names > 96
                    || (long != 0 && inline != 0)
                    || long > MAX_VALUE_BYTES
                {
                    return Err(Code::InvalidRequest);
                }
                let token = u64at(body, 0);
                let index = self.key(channel, peer, token)?;
                let mut name = [0; 48];
                let name = units(&body[header..header + names], &mut name)?;
                let staged = long != 0;
                if staged
                    && (self.value_stage.token != token
                        || self.value_stage.total != long
                        || self.value_stage.received != long)
                {
                    return Err(Code::InvalidRequest);
                }
                let data: &[u8] = if staged {
                    &self.value_stage.bytes[..long]
                } else {
                    &body[header + names..]
                };
                let value_kind = kind(body[8])?;
                Value::check(value_kind, data).map_err(persistence_error)?;
                if operation == opcode::REGISTRY_SET_CHECKED
                    && self
                        .volume
                        .registry_version(&self.keys[index].as_ref().unwrap().key)
                        .map_err(persistence_error)?
                        != u64at(body, 16)
                {
                    return Err(Code::Changed);
                }
                let set = self.volume.registry_set_data(
                    &self.keys[index].as_ref().unwrap().key,
                    name,
                    value_kind,
                    data,
                );
                if staged {
                    self.value_stage.token = 0;
                }
                set.map_err(persistence_error)?;
                Ok(Response::empty())
            }
            // [key u64][offset u16][whole length u16][0; 4][bytes]: the
            // stage belongs to one key until its value is set (or the key
            // closes); a first part (offset 0) restarts it.
            opcode::REGISTRY_STAGE_VALUE => {
                if body.len() <= 16 || body[12..16] != [0; 4] {
                    return Err(Code::InvalidRequest);
                }
                let token = u64at(body, 0);
                self.key(channel, peer, token)?;
                let offset = usize::from(u16::from_le_bytes(body[8..10].try_into().unwrap()));
                let total = usize::from(u16::from_le_bytes(body[10..12].try_into().unwrap()));
                let bytes = &body[16..];
                let stage = &mut self.value_stage;
                if stage.token != 0 && stage.token != token {
                    // Another key's value is mid-stage (its client retries).
                    if self.keys.iter().flatten().any(|k| k.token == stage.token) {
                        return Err(Code::WouldBlock);
                    }
                    stage.token = 0;
                }
                if total == 0 || total > MAX_VALUE_BYTES || offset + bytes.len() > total {
                    return Err(Code::InvalidRequest);
                }
                if offset == 0 {
                    stage.token = token;
                    stage.total = total;
                    stage.received = 0;
                }
                if stage.token != token || stage.total != total || stage.received != offset {
                    return Err(Code::InvalidRequest);
                }
                stage.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
                stage.received += bytes.len();
                Ok(Response::empty())
            }
            opcode::REGISTRY_DELETE_VALUE => {
                if body.len() < 16
                    || body[10..16] != [0; 6]
                    || body.len()
                        != 16 + usize::from(u16::from_le_bytes(body[8..10].try_into().unwrap())) * 2
                {
                    return Err(Code::InvalidRequest);
                }
                let index = self.key(channel, peer, u64at(body, 0))?;
                let mut name = [0; 48];
                let name = units(&body[16..], &mut name)?;
                self.volume
                    .registry_delete_value(&self.keys[index].as_ref().unwrap().key, name)
                    .map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::REGISTRY_DELETE_KEY if body.len() == 8 => {
                let index = self.key(channel, peer, u64at(body, 0))?;
                self.volume
                    .registry_delete_key(&self.keys[index].as_ref().unwrap().key)
                    .map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::REGISTRY_CLOSE if body.len() == 8 => {
                let index = self.key(channel, peer, u64at(body, 0))?;
                self.volume
                    .registry_close(self.keys[index].take().unwrap().key)
                    .map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::REGISTRY_VERSION if body.len() == 8 => {
                let index = self.key(channel, peer, u64at(body, 0))?;
                let version = self
                    .volume
                    .registry_version(&self.keys[index].as_ref().unwrap().key)
                    .map_err(persistence_error)?;
                Ok(Response::scalar(version))
            }
            opcode::REGISTRY_ENUM_KEY | opcode::REGISTRY_ENUM_VALUE => {
                let (token, observed, position) = enumeration_request(body)?;
                let index = self.key(channel, peer, token)?;
                let key = &self.keys[index].as_ref().unwrap().key;
                let (name, value) = if operation == opcode::REGISTRY_ENUM_KEY {
                    (
                        self.volume
                            .registry_enumerate_key(key, position, observed)
                            .map_err(persistence_error)?,
                        None,
                    )
                } else {
                    let (name, value) = self
                        .volume
                        .registry_enumerate_value(key, position, observed)
                        .map_err(persistence_error)?;
                    (name, Some(value))
                };
                Ok(enumerated(name.units(), value))
            }
            _ => Err(Code::InvalidRequest),
        }
    }
}
fn enumeration_request(body: &[u8]) -> Result<(u64, u64, usize), Code> {
    if body.len() != 24 || body[20..24] != [0; 4] {
        return Err(Code::InvalidRequest);
    }
    Ok((
        u64at(body, 0),
        u64at(body, 8),
        u32::from_le_bytes(body[16..20].try_into().unwrap()) as usize,
    ))
}
fn enumerated(name: &[u16], value: Option<Value>) -> Response {
    let mut response = Response::empty();
    response.body[0] = name.len() as u8;
    for (unit, bytes) in name.iter().zip(response.body[8..].chunks_exact_mut(2)) {
        bytes.copy_from_slice(&unit.to_le_bytes());
    }
    response.len = 8 + name.len() * 2;
    if let Some(value) = value {
        // The first record's bytes and the whole length: a client reads
        // the rest of a longer value with REGISTRY_QUERY.
        response.body[1] = value.kind as u8;
        response.body[2] = value.data().len() as u8;
        response.body[4..6].copy_from_slice(&(value.total() as u16).to_le_bytes());
        response.body[response.len..response.len + value.data().len()]
            .copy_from_slice(value.data());
        response.len += value.data().len();
    }
    response
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enumeration_has_exact_request_bounds_and_lossless_maximum_reply() {
        let mut body = [0; 24];
        body[..8].copy_from_slice(&91u64.to_le_bytes());
        body[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        body[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            enumeration_request(&body),
            Ok((91, u64::MAX, u32::MAX as usize))
        );
        for length in 0..24 {
            assert_eq!(
                enumeration_request(&body[..length]),
                Err(Code::InvalidRequest)
            );
        }
        for index in 20..24 {
            let mut bad = body;
            bad[index] = 1;
            assert_eq!(enumeration_request(&bad), Err(Code::InvalidRequest));
        }
        assert_eq!(enumeration_request(&[0; 25]), Err(Code::InvalidRequest));
        let mut name = [65; 48];
        name[47] = 0xd800;
        let r = enumerated(
            &name,
            Some(Value::new(RegistryType::Binary, &[0xa5; 80]).unwrap()),
        );
        assert_eq!(r.len, 184);
        assert_eq!(&r.body[..8], &[48, 3, 80, 0, 80, 0, 0, 0]);
        assert_eq!(&r.body[102..104], &0xd800u16.to_le_bytes());
        assert_eq!(&r.body[104..184], &[0xa5; 80]);
        let r = enumerated(
            &[],
            Some(Value::new(RegistryType::String, &[0, 0]).unwrap()),
        );
        assert_eq!(r.len, 10);
        assert_eq!(&r.body[..10], &[0, 1, 2, 0, 2, 0, 0, 0, 0, 0]);
    }
    #[test]
    fn lossless_utf16_and_wire_type_bounds_do_not_normalize_names() {
        let mut output = [0; 3];
        assert_eq!(
            units(&[0, 0xd8, 0xff, 0xdf, 0, 0], &mut output),
            Ok(&[0xd800, 0xdfff, 0][..])
        );
        assert_eq!(units(&[1], &mut output), Err(Code::InvalidRequest));
        assert_eq!(units(&[0; 8], &mut output), Err(Code::InvalidRequest));
        for value in 0..=255 {
            assert_eq!(kind(value).is_ok(), [0, 1, 2, 3, 4, 7, 11].contains(&value));
        }
    }
}
