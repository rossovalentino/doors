use super::*;
/// A request path: native (`/...`), or a Windows path (`C:\...`, UTF-8)
/// that storage resolves the Windows way (ASCII case folded, `\` or `/`
/// separators) into `scratch`. Native paths always start with `/`, so the
/// two never meet.
pub(super) fn path<'a>(bytes: &'a [u8], scratch: &'a mut [u16]) -> Result<Path<'a>, Code> {
    if bytes.is_empty() || bytes.len() > PATH_BYTES || bytes.contains(&0) {
        return Err(Code::InvalidRequest);
    }
    let text = core::str::from_utf8(bytes).map_err(|_| Code::InvalidRequest)?;
    if bytes[0] == b'/' {
        return Ok(Path::Native(text));
    }
    if !(bytes.len() >= 3
        && bytes[0].eq_ignore_ascii_case(&b'c')
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/'))
    {
        return Err(Code::InvalidRequest);
    }
    let mut count = 0;
    for unit in text.encode_utf16() {
        *scratch.get_mut(count).ok_or(Code::InvalidRequest)? = unit;
        count += 1;
    }
    Ok(Path::Windows(&scratch[..count]))
}
pub(super) fn info(value: door_storage_core::persistence::FileInfo) -> [u8; 32] {
    door_service_protocol::files::FileInfo {
        attributes: value.attributes,
        bytes: value.bytes,
        object: value.object,
        version: value.version,
        directory: value.directory,
    }
    .encode()
}
/// METADATA / FILE_INFO / METADATA_FOR reply: info then catalog timestamps.
pub(super) fn stat(
    value: door_storage_core::persistence::FileInfo,
) -> [u8; door_service_protocol::files::STAT_BYTES] {
    let mut b = [0; door_service_protocol::files::STAT_BYTES];
    b[..32].copy_from_slice(&info(value));
    b[32..].copy_from_slice(&times(value.times));
    b
}
fn times(
    t: door_storage_core::persistence::FileTimes,
) -> [u8; door_service_protocol::files::TIMES_BYTES] {
    door_service_protocol::files::FileTimes {
        created: t.created,
        modified: t.modified,
        accessed: t.accessed,
    }
    .encode()
}
impl Broker<'_> {
    pub(super) fn file_operations(
        &mut self,
        channel: Handle,
        peer: Identity,
        subject: Subject,
        operation: u32,
        body: &[u8],
    ) -> Result<Response, Code> {
        match operation {
            opcode::METADATA => {
                let value = self
                    .volume
                    .metadata(subject, path(body, &mut [0; PATH_BYTES])?, Context::root())
                    .map_err(persistence_error)?;
                Ok(Response::bytes(&stat(value)))
            }
            opcode::FILE_INFO if body.len() == 8 => {
                let i = self.file(channel, peer, u64at(body, 0))?;
                let value = self
                    .volume
                    .info(&self.files[i].as_ref().unwrap().file)
                    .map_err(persistence_error)?;
                Ok(Response::bytes(&stat(value)))
            }
            opcode::LIST if body.len() > 8 => {
                let entry = self
                    .volume
                    .list(
                        subject,
                        path(&body[8..], &mut [0; PATH_BYTES])?,
                        Context::root(),
                        u64at(body, 0),
                    )
                    .map_err(persistence_error)?;
                let Some(entry) = entry else {
                    return Ok(Response::empty());
                };
                use door_service_protocol::files::{LIST_NAME_AT, LIST_NAME_UNITS, STAT_BYTES};
                let mut reply = Response::empty();
                let units = entry.name.units();
                // The first units; a longer name's rest comes by NAME_PART.
                let here = units.len().min(LIST_NAME_UNITS);
                reply.body[..STAT_BYTES].copy_from_slice(&stat(entry.info));
                reply.body[STAT_BYTES] = units.len() as u8;
                for (out, unit) in reply.body[LIST_NAME_AT..]
                    .chunks_exact_mut(2)
                    .zip(&units[..here])
                {
                    out.copy_from_slice(&unit.to_le_bytes());
                }
                reply.len = LIST_NAME_AT + here * 2;
                Ok(reply)
            }
            // The rest of a listed name: the entry relisted by its object
            // ID (body: object, unit offset, 6 reserved bytes, the folder).
            opcode::NAME_PART if body.len() > 16 => {
                use door_service_protocol::files::NAME_PART_UNITS;
                let object = u64at(body, 0);
                let offset = usize::from(u16::from_le_bytes([body[8], body[9]]));
                if body[10..16].iter().any(|b| *b != 0) {
                    return Err(Code::InvalidRequest);
                }
                let entry = self
                    .volume
                    .list(
                        subject,
                        path(&body[16..], &mut [0; PATH_BYTES])?,
                        Context::root(),
                        object.checked_sub(1).ok_or(Code::InvalidRequest)?,
                    )
                    .map_err(persistence_error)?
                    .filter(|e| e.info.object == object)
                    .ok_or(Code::NotFound)?;
                let units = entry.name.units();
                if offset == 0 || offset >= units.len() {
                    return Err(Code::InvalidRequest);
                }
                let n = (units.len() - offset).min(NAME_PART_UNITS);
                let mut reply = Response::empty();
                for (out, unit) in reply
                    .body
                    .chunks_exact_mut(2)
                    .zip(&units[offset..offset + n])
                {
                    out.copy_from_slice(&unit.to_le_bytes());
                }
                reply.len = n * 2;
                Ok(reply)
            }
            opcode::MKDIR => {
                self.volume
                    .mkdir(
                        subject,
                        path(body, &mut [0; PATH_BYTES])?,
                        Context::root(),
                        None,
                    )
                    .map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::REMOVE => {
                self.volume
                    .remove_path(subject, path(body, &mut [0; PATH_BYTES])?, Context::root())
                    .map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::RENAME if body.len() >= 2 => {
                let length = usize::from(u16::from_le_bytes(body[..2].try_into().unwrap()));
                if length > body.len() - 2 {
                    return Err(Code::InvalidRequest);
                }
                self.volume
                    .rename_path(
                        subject,
                        path(&body[2..2 + length], &mut [0; PATH_BYTES])?,
                        path(&body[2 + length..], &mut [0; PATH_BYTES])?,
                        Context::root(),
                    )
                    .map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::RENAME_CHILD_CHECKED => {
                let request = door_service_protocol::files::RenameChild::decode(body)?;
                self.volume
                    .rename_child_checked(
                        subject,
                        request.parent(),
                        request.object(),
                        request.version(),
                        request.name(),
                    )
                    .map_err(persistence_error)?;
                self.volume.commit().map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::REMOVE_CHILD_CHECKED => {
                let r = door_service_protocol::files::RemoveChild::decode(body)?;
                self.volume
                    .check_remove_child(subject, r.parent(), r.object(), r.version())
                    .map_err(persistence_error)?;
                // Isolate existing staging first, then recheck the observation;
                // no later request can commit part of an unsuccessful deletion.
                self.volume.commit().map_err(persistence_error)?;
                self.volume
                    .remove_child_checked(subject, r.parent(), r.object(), r.version())
                    .map_err(persistence_error)?;
                self.volume.commit().map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::TRUNCATE if body.len() == 16 => {
                let i = self.file(channel, peer, u64at(body, 0))?;
                if self.ordinary {
                    let mut length = self
                        .volume
                        .info(&self.files[i].as_ref().unwrap().file)
                        .map_err(persistence_error)?
                        .bytes;
                    let wanted = u64at(body, 8);
                    if wanted > 4_243_456 {
                        return Err(Code::Resources);
                    }
                    while wanted > length.saturating_add(4096) {
                        length += 4096;
                        self.volume
                            .truncate(&self.files[i].as_ref().unwrap().file, length)
                            .map_err(persistence_error)?;
                        self.volume.commit().map_err(persistence_error)?;
                    }
                }
                self.volume
                    .truncate(&self.files[i].as_ref().unwrap().file, u64at(body, 8))
                    .map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::OPEN_FOR | opcode::METADATA_FOR if body.len() > 4 => {
                if !trusted_init(peer) {
                    return Err(Code::Denied);
                }
                let uid = u32::from_le_bytes(body[..4].try_into().unwrap());
                // The local accounts, and the service account Init starts
                // services as.
                if ![1001, 1002, door_abi::platform::SERVICE_ACCOUNT].contains(&uid) {
                    return Err(Code::Denied);
                }
                let delegated =
                    unsafe { Subject::authenticated(uid, 0, 1) }.map_err(persistence_error)?;
                if operation == opcode::METADATA_FOR {
                    let value = self
                        .volume
                        .metadata(
                            delegated,
                            path(&body[4..], &mut [0; PATH_BYTES])?,
                            Context::root(),
                        )
                        .map_err(persistence_error)?;
                    return Ok(Response::bytes(&stat(value)));
                }
                let i = self
                    .files
                    .iter()
                    .position(Option::is_none)
                    .ok_or(Code::Resources)?;
                let next = self.next.checked_add(1).ok_or(Code::Resources)?;
                let (file, _) = self
                    .volume
                    .open(
                        delegated,
                        path(&body[4..], &mut [0; PATH_BYTES])?,
                        Context::root(),
                        OpenOptions {
                            access: Access::READ,
                            share: Access::SHARE_ALL,
                            disposition: Disposition::OpenExisting,
                        },
                    )
                    .map_err(persistence_error)?;
                let token = self.next;
                self.next = next;
                self.files[i] = Some(Open {
                    channel,
                    pid: peer.pid,
                    session: peer.session,
                    epoch: peer.epoch,
                    token,
                    file,
                });
                Ok(Response::scalar(token))
            }
            opcode::SECURITY => {
                let (acl, directory) = self
                    .volume
                    .security(subject, path(body, &mut [0; PATH_BYTES])?, Context::root())
                    .map_err(persistence_error)?;
                Ok(Response::bytes(&security(acl, directory).encode()))
            }
            opcode::FILE_SECURITY if body.len() == 8 => {
                let i = self.file(channel, peer, u64at(body, 0))?;
                let (acl, directory) = self
                    .volume
                    .security_of(subject, &self.files[i].as_ref().unwrap().file)
                    .map_err(persistence_error)?;
                Ok(Response::bytes(&security(acl, directory).encode()))
            }
            opcode::SET_SECURITY if body.len() > SECURITY_BYTES => {
                let acl = acl_of(&Security::decode(&body[..SECURITY_BYTES])?)?;
                self.volume
                    .set_security(
                        subject,
                        path(&body[SECURITY_BYTES..], &mut [0; PATH_BYTES])?,
                        Context::root(),
                        acl,
                    )
                    .map_err(persistence_error)?;
                self.volume.commit().map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::SET_FILE_SECURITY if body.len() == 8 + SECURITY_BYTES => {
                let i = self.file(channel, peer, u64at(body, 0))?;
                let acl = acl_of(&Security::decode(&body[8..])?)?;
                self.volume
                    .set_security_of(subject, &self.files[i].as_ref().unwrap().file, acl)
                    .map_err(persistence_error)?;
                self.volume.commit().map_err(persistence_error)?;
                Ok(Response::empty())
            }
            _ => Err(Code::InvalidRequest),
        }
    }
}
use door_service_protocol::files::{SECURITY_BYTES, Security, SecurityEntry, principal};
/// Storage's ACL as the SECURITY record.
fn security(acl: Acl, directory: bool) -> Security {
    use door_storage_core::persistence::Principal;
    let mut out = Security {
        owner: acl.owner,
        integrity: acl.integrity,
        directory,
        count: acl.entries().len() as u8,
        ..Security::default()
    };
    for (slot, ace) in out.entries.iter_mut().zip(acl.entries()) {
        let (kind, id) = match ace.principal {
            Principal::Everyone => (principal::EVERYONE, 0),
            Principal::User(uid) => (principal::USER, uid),
            Principal::Group(bit) => (principal::GROUP, u32::from(bit)),
        };
        *slot = SecurityEntry {
            principal: kind,
            id,
            deny: ace.deny,
            access: ace.access.0,
            inherit: ace.inherit,
        };
    }
    out
}
/// A SET_SECURITY record as storage's ACL (the directory flag is ignored).
fn acl_of(record: &Security) -> Result<Acl, Code> {
    use door_storage_core::persistence::{Ace, Principal};
    let mut acl = Acl::empty(record.owner, record.integrity);
    for e in record.entries() {
        acl.push(Ace {
            principal: match e.principal {
                principal::EVERYONE => Principal::Everyone,
                principal::USER => Principal::User(e.id),
                _ => Principal::Group(e.id as u8),
            },
            deny: e.deny,
            access: Access(e.access),
            inherit: e.inherit,
        })
        .map_err(|_| Code::InvalidRequest)?;
    }
    Ok(acl)
}
