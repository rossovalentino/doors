use super::*;
/// A path request's body: a short prefix and a path.
const PATH_BODY: usize = 16 + PATH_BYTES;
impl Storage {
    pub fn disk_info(&mut self) -> Result<door_service_protocol::administration::DiskInfo> {
        self.operation(|storage| {
            let reply = storage.connection.call(opcode::DISK_INFO, &[], &[])?;
            door_service_protocol::administration::DiskInfo::decode(reply.data()?)
                .map_err(|_| Error::Malformed)
        })
    }
    /// Total and free bytes of the home volume (C: and the user tree).
    pub fn volume_usage(&mut self) -> Result<door_service_protocol::administration::VolumeUsage> {
        self.operation(|storage| {
            let reply = storage.connection.call(opcode::VOLUME_USAGE, &[], &[])?;
            door_service_protocol::administration::VolumeUsage::decode(reply.data()?)
                .map_err(|_| Error::Malformed)
        })
    }
    /// Trusted init broker metadata lookup using the authenticated user's ACLs.
    pub fn metadata_for(&mut self, uid: u32, path: &str) -> Result<FileInfo> {
        self.operation(|storage| {
            if path.len() > PATH_BYTES {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0; PATH_BYTES + 4];
            body[..4].copy_from_slice(&uid.to_le_bytes());
            body[4..4 + path.len()].copy_from_slice(path.as_bytes());
            let reply = storage.long_call(opcode::METADATA_FOR, &body[..4 + path.len()])?;
            stat_reply(reply.data()?).map(|(info, _)| info)
        })
    }
    pub fn connect_port(port: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(port, manifest::STORAGE)?,
        })
    }
    pub fn metadata(&mut self, path: &str) -> Result<FileInfo> {
        self.stat(path).map(|(info, _)| info)
    }
    /// Metadata with the volume timestamps (0 = not recorded).
    pub fn stat(&mut self, path: &str) -> Result<(FileInfo, FileTimes)> {
        self.operation(|storage| {
            let reply = storage.long_call(opcode::METADATA, path.as_bytes())?;
            stat_reply(reply.data()?)
        })
    }
    pub fn info(&mut self, file: &File) -> Result<FileInfo> {
        self.file_stat(file).map(|(info, _)| info)
    }
    /// A path's access control list (GetNamedSecurityInfo).
    pub fn security(&mut self, path: &str) -> Result<door_service_protocol::files::Security> {
        self.operation(|storage| {
            let reply = storage.long_call(opcode::SECURITY, path.as_bytes())?;
            door_service_protocol::files::Security::decode(reply.data()?)
                .map_err(|_| Error::Malformed)
        })
    }
    /// An open file's access control list (GetSecurityInfo).
    pub fn file_security(&mut self, file: &File) -> Result<door_service_protocol::files::Security> {
        self.operation(|storage| {
            storage.check_file(file)?;
            let reply =
                storage
                    .connection
                    .call(opcode::FILE_SECURITY, &file.token.to_le_bytes(), &[])?;
            door_service_protocol::files::Security::decode(reply.data()?)
                .map_err(|_| Error::Malformed)
        })
    }
    /// Replace a path's access control list (SetNamedSecurityInfo).
    pub fn set_security(
        &mut self,
        path: &str,
        record: &door_service_protocol::files::Security,
    ) -> Result<()> {
        use door_service_protocol::files::SECURITY_BYTES;
        self.operation(|storage| {
            if path.len() > PATH_BYTES {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0; PATH_BYTES + SECURITY_BYTES];
            body[..SECURITY_BYTES].copy_from_slice(&record.encode());
            body[SECURITY_BYTES..SECURITY_BYTES + path.len()].copy_from_slice(path.as_bytes());
            storage.long_call(opcode::SET_SECURITY, &body[..SECURITY_BYTES + path.len()])?;
            Ok(())
        })
    }
    /// Replace an open file's access control list (SetSecurityInfo).
    pub fn set_file_security(
        &mut self,
        file: &File,
        record: &door_service_protocol::files::Security,
    ) -> Result<()> {
        use door_service_protocol::files::SECURITY_BYTES;
        self.operation(|storage| {
            storage.check_file(file)?;
            let mut body = [0; 8 + SECURITY_BYTES];
            body[..8].copy_from_slice(&file.token.to_le_bytes());
            body[8..].copy_from_slice(&record.encode());
            storage
                .connection
                .call(opcode::SET_FILE_SECURITY, &body, &[])?;
            Ok(())
        })
    }
    pub fn file_stat(&mut self, file: &File) -> Result<(FileInfo, FileTimes)> {
        self.operation(|storage| {
            storage.check_file(file)?;
            let reply =
                storage
                    .connection
                    .call(opcode::FILE_INFO, &file.token.to_le_bytes(), &[])?;
            stat_reply(reply.data()?)
        })
    }
    pub fn list(&mut self, path: &str, after_object: u64) -> Result<Option<DirEntry>> {
        self.operation(|storage| {
            if path.len() > PATH_BYTES {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0; PATH_BYTES + 8];
            body[..8].copy_from_slice(&after_object.to_le_bytes());
            body[8..8 + path.len()].copy_from_slice(path.as_bytes());
            let reply = storage.long_call(opcode::LIST, &body[..8 + path.len()])?;
            let Some(mut entry) = decode_entry(reply.data()?, after_object)? else {
                return Ok(None);
            };
            // A name past the first reply's units: the rest in NAME_PART
            // replies (the entry relisted by its object ID).
            let mut have = entry
                .name_len
                .min(door_service_protocol::files::LIST_NAME_UNITS);
            while have < entry.name_len {
                let mut part = [0; PATH_BYTES + 16];
                part[..8].copy_from_slice(&entry.info.object.to_le_bytes());
                part[8..10].copy_from_slice(&(have as u16).to_le_bytes());
                part[16..16 + path.len()].copy_from_slice(path.as_bytes());
                let reply = storage.long_call(opcode::NAME_PART, &part[..16 + path.len()])?;
                let units = reply.data()?;
                let n = units.len() / 2;
                if n == 0 || units.len() % 2 != 0 || have + n > entry.name_len {
                    return Err(Error::Malformed);
                }
                for (i, b) in units.chunks_exact(2).enumerate() {
                    entry.name[have + i] = u16::from_le_bytes([b[0], b[1]]);
                }
                have += n;
            }
            if !valid_name(&entry.name[..entry.name_len]) {
                return Err(Error::Malformed);
            }
            Ok(Some(entry))
        })
    }
    pub fn mkdir(&mut self, path: &str) -> Result<()> {
        self.operation(|storage| storage.long_call(opcode::MKDIR, path.as_bytes())?.empty())
    }
    pub fn remove(&mut self, path: &str) -> Result<()> {
        self.operation(|storage| storage.long_call(opcode::REMOVE, path.as_bytes())?.empty())
    }
    /// Durable permanent deletion of the exact observed child binding.
    pub fn remove_child_checked(&mut self, parent: u64, object: u64, version: u64) -> Result<()> {
        self.operation(|storage| {
            let request = door_service_protocol::files::RemoveChild::new(parent, object, version)
                .map_err(Error::Service)?;
            storage
                .connection
                .call(opcode::REMOVE_CHILD_CHECKED, &request.encode(), &[])?
                .empty()
        })
    }
    pub fn rename(&mut self, old: &str, new: &str) -> Result<()> {
        self.operation(|storage| {
            if old.len() > PATH_BYTES || new.len() > PATH_BYTES {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0; 2 + 2 * PATH_BYTES];
            body[..2].copy_from_slice(&(old.len() as u16).to_le_bytes());
            body[2..2 + old.len()].copy_from_slice(old.as_bytes());
            body[2 + old.len()..2 + old.len() + new.len()].copy_from_slice(new.as_bytes());
            storage
                .long_call(opcode::RENAME, &body[..2 + old.len() + new.len()])?
                .empty()
        })
    }
    /// A successful reply includes the storage owner's durable journal commit.
    pub fn rename_child_checked(
        &mut self,
        parent: u64,
        object: u64,
        version: u64,
        name: &[u16],
    ) -> Result<()> {
        self.operation(|storage| {
            let request =
                door_service_protocol::files::RenameChild::new(parent, object, version, name)
                    .map_err(Error::Service)?;
            storage
                .connection
                .call(opcode::RENAME_CHILD_CHECKED, &request.encode(), &[])?
                .empty()
        })
    }
    pub fn truncate(&mut self, file: &File, bytes: u64) -> Result<()> {
        self.operation(|storage| {
            storage.check_file(file)?;
            let mut body = [0; 16];
            body[..8].copy_from_slice(&file.token.to_le_bytes());
            body[8..].copy_from_slice(&bytes.to_le_bytes());
            storage
                .connection
                .call(opcode::TRUNCATE, &body, &[])?
                .empty()
        })
    }
    /// Trusted init broker delegation. Storage rejects every other manifest.
    pub fn open_for(&mut self, uid: u32, path: &str) -> Result<File> {
        self.operation(|storage| {
            if path.len() > PATH_BYTES {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0; PATH_BYTES + 4];
            body[..4].copy_from_slice(&uid.to_le_bytes());
            body[4..4 + path.len()].copy_from_slice(path.as_bytes());
            let token = storage
                .long_call(opcode::OPEN_FOR, &body[..4 + path.len()])?
                .token()?;
            Ok(File {
                token,
                owner: storage.connection.key(),
            })
        })
    }
    /// Bounded transactions permit files larger than the journal working set.
    /// Earlier chunks remain committed if a later chunk fails.
    pub fn write_all(&mut self, file: &File, mut offset: u64, input: &[u8]) -> Result<()> {
        self.operation(|storage| {
            for transaction in input.chunks(4096) {
                for chunk in transaction.chunks(CHUNK_BYTES) {
                    let n = storage.write(file, offset, chunk)?;
                    if n != chunk.len() {
                        return Err(Error::Service(Code::Io));
                    }
                    offset = offset
                        .checked_add(n as u64)
                        .ok_or(Error::Service(Code::Resources))?;
                }
                storage.flush()?;
            }
            Ok(())
        })
    }
    pub fn append(&mut self, file: &File, input: &[u8]) -> Result<()> {
        self.operation(|storage| {
            let offset = storage.info(file)?.bytes;
            storage.write_all(file, offset, input)
        })
    }
    // ---- P14 file operations (copy/move/permanent delete) ----
    fn path_body<'b>(body: &'b mut [u8; PATH_BODY], prefix: &[u8], path: &str) -> Result<&'b [u8]> {
        if path.len() > PATH_BYTES || path.as_bytes().contains(&0) {
            return Err(Error::Service(Code::InvalidRequest));
        }
        body[..prefix.len()].copy_from_slice(prefix);
        body[prefix.len()..prefix.len() + path.len()].copy_from_slice(path.as_bytes());
        Ok(&body[..prefix.len() + path.len()])
    }
    fn progress(reply: &[u8]) -> Result<OpProgress> {
        OpProgress::decode(reply).map_err(|_| Error::Malformed)
    }
    /// Plan an operation into folder `destination` ("" for Delete). Nothing
    /// changes until `fileop_step`. Returns the operation ID.
    pub fn fileop_begin(
        &mut self,
        mode: FileOpMode,
        destination: &str,
        policy: ConflictPolicy,
    ) -> Result<u64> {
        self.operation(|storage| {
            let mut body = [0; PATH_BODY];
            let body = Self::path_body(&mut body, &[mode as u8, policy as u8], destination)?;
            storage.long_call(opcode::FILEOP_BEGIN, body)?.token()
        })
    }
    /// Add one source (all sources share one folder; at most 32).
    pub fn fileop_add(&mut self, op: u64, source: &str) -> Result<()> {
        self.operation(|storage| {
            let mut body = [0; PATH_BODY];
            let body = Self::path_body(&mut body, &op.to_le_bytes(), source)?;
            storage.long_call(opcode::FILEOP_ADD, body)?.empty()
        })
    }
    /// Advance by one bounded, durable step.
    pub fn fileop_step(&mut self, op: u64) -> Result<OpProgress> {
        self.op_call(opcode::FILEOP_STEP, &op.to_le_bytes())
    }
    pub fn fileop_query(&mut self, op: u64) -> Result<OpProgress> {
        self.op_call(opcode::FILEOP_QUERY, &op.to_le_bytes())
    }
    fn op_call(&mut self, code: u32, body: &[u8]) -> Result<OpProgress> {
        self.operation(|storage| {
            let reply = storage.connection.call(code, body, &[])?;
            Self::progress(reply.data()?)
        })
    }
    /// Answer the pending conflict; `apply_to_all` keeps the decision for
    /// the rest of the operation.
    pub fn fileop_resolve(
        &mut self,
        op: u64,
        decision: Decision,
        apply_to_all: bool,
    ) -> Result<()> {
        self.operation(|storage| {
            let mut body = [0; 10];
            body[..8].copy_from_slice(&op.to_le_bytes());
            body[8] = decision as u8;
            body[9] = u8::from(apply_to_all);
            storage
                .connection
                .call(opcode::FILEOP_RESOLVE, &body, &[])?
                .empty()
        })
    }
    fn control(&mut self, op: u64, action: OpControl) -> Result<Option<OpProgress>> {
        self.operation(|storage| {
            let mut body = [0; 9];
            body[..8].copy_from_slice(&op.to_le_bytes());
            body[8] = action as u8;
            let reply = storage
                .connection
                .call(opcode::FILEOP_CONTROL, &body, &[])?;
            let data = reply.data()?;
            if data.is_empty() {
                Ok(None)
            } else {
                Self::progress(data).map(Some)
            }
        })
    }
    /// Durable pause; steps then do nothing until `fileop_resume`.
    pub fn fileop_pause(&mut self, op: u64) -> Result<OpProgress> {
        self.control(op, OpControl::Pause)?.ok_or(Error::Malformed)
    }
    pub fn fileop_resume(&mut self, op: u64) -> Result<OpProgress> {
        self.control(op, OpControl::Resume)?.ok_or(Error::Malformed)
    }
    /// Discard the partial item; finished items stay.
    pub fn fileop_cancel(&mut self, op: u64) -> Result<OpProgress> {
        self.control(op, OpControl::Cancel)?.ok_or(Error::Malformed)
    }
    /// Forget an operation (an unfinished one is cancelled first).
    pub fn fileop_dismiss(&mut self, op: u64) -> Result<()> {
        self.control(op, OpControl::Dismiss).map(|_| ())
    }
    /// The caller's operations after cursor `after` (including ones a reboot
    /// interrupted, which come back Paused).
    pub fn fileop_list(&mut self, after: u64) -> Result<Option<OpProgress>> {
        self.operation(|storage| {
            let reply = storage
                .connection
                .call(opcode::FILEOP_LIST, &after.to_le_bytes(), &[])?;
            let data = reply.data()?;
            if data.is_empty() {
                return Ok(None);
            }
            let p = Self::progress(data)?;
            if p.op <= after {
                return Err(Error::Malformed);
            }
            Ok(Some(p))
        })
    }
    // ---- Recycle Bin ----
    /// Move `path` to the caller's Recycle Bin (one durable transaction).
    pub fn recycle(&mut self, path: &str) -> Result<RecycleOutcome> {
        self.operation(|storage| {
            let mut body = [0; PATH_BODY];
            let body = Self::path_body(&mut body, &[], path)?;
            let reply = storage.long_call(opcode::RECYCLE, body)?;
            RecycleOutcome::decode(reply.data()?).map_err(|_| Error::Malformed)
        })
    }
    /// Bin entries in ascending ID order after `after`.
    pub fn recycle_list(&mut self, after: u64) -> Result<Option<RecycleEntry>> {
        self.operation(|storage| {
            let reply = storage
                .connection
                .call(opcode::RECYCLE_LIST, &after.to_le_bytes(), &[])?;
            let data = reply.data()?;
            if data.is_empty() {
                return Ok(None);
            }
            let entry = RecycleEntry::decode(data).map_err(|_| Error::Malformed)?;
            if entry.id <= after {
                return Err(Error::Malformed);
            }
            Ok(Some(entry))
        })
    }
    /// Put an item back at its original path (`Exists` if that name is taken).
    pub fn recycle_restore(&mut self, id: u64) -> Result<()> {
        self.operation(|storage| {
            storage
                .connection
                .call(opcode::RECYCLE_RESTORE, &id.to_le_bytes(), &[])?
                .empty()
        })
    }
    /// Delete one entry permanently: returns a Delete operation to step.
    pub fn recycle_purge(&mut self, id: u64) -> Result<u64> {
        if id == 0 {
            return Err(Error::Service(Code::InvalidRequest));
        }
        self.purge(id)
    }
    /// Empty the bin: returns a Delete operation to step (0 = already empty).
    pub fn recycle_empty(&mut self) -> Result<u64> {
        self.purge(0)
    }
    fn purge(&mut self, id: u64) -> Result<u64> {
        self.operation(|storage| {
            storage
                .connection
                .call(opcode::RECYCLE_PURGE, &id.to_le_bytes(), &[])?
                .scalar()
        })
    }
    pub fn recycle_usage(&mut self) -> Result<RecycleUsage> {
        self.operation(|storage| {
            let reply = storage.connection.call(opcode::RECYCLE_USAGE, &[], &[])?;
            RecycleUsage::decode(reply.data()?).map_err(|_| Error::Malformed)
        })
    }
    // ---- Change notification ----
    /// Subscribe to changes of `folder`'s children (any process). Returns a
    /// token bound to this connection.
    pub fn watch(&mut self, folder: &str) -> Result<u64> {
        self.operation(|storage| {
            let mut body = [0; PATH_BODY];
            let body = Self::path_body(&mut body, &[WATCH_FOLDER], folder)?;
            storage.long_call(opcode::WATCH, body)?.token()
        })
    }
    /// Subscribe to the caller's settings and registry changes (and
    /// machine-wide ones).
    pub fn watch_settings(&mut self) -> Result<u64> {
        self.operation(|storage| {
            storage
                .connection
                .call(opcode::WATCH, &[WATCH_SETTINGS], &[])?
                .token()
        })
    }
    /// Cheap: whether anything changed since the previous poll.
    pub fn watch_poll(&mut self, watch: u64) -> Result<WatchState> {
        self.operation(|storage| {
            let reply = storage
                .connection
                .call(opcode::WATCH_POLL, &watch.to_le_bytes(), &[])?;
            WatchState::decode(reply.data()?).map_err(|_| Error::Malformed)
        })
    }
    pub fn unwatch(&mut self, watch: u64) -> Result<()> {
        self.operation(|storage| {
            storage
                .connection
                .call(opcode::UNWATCH, &watch.to_le_bytes(), &[])?
                .empty()
        })
    }
}
use door_service_protocol::files::{
    ConflictPolicy, Decision, FileOpMode, OpControl, OpProgress, RecycleEntry, RecycleOutcome,
    RecycleUsage, WATCH_FOLDER, WATCH_SETTINGS, WatchState,
};

fn stat_reply(bytes: &[u8]) -> Result<(FileInfo, FileTimes)> {
    door_service_protocol::files::decode_stat(bytes).map_err(|_| Error::Malformed)
}
fn decode_entry(bytes: &[u8], after_object: u64) -> Result<Option<DirEntry>> {
    use door_service_protocol::files::{LIST_NAME_AT, LIST_NAME_UNITS, NAME_UNITS, STAT_BYTES};
    let b = bytes;
    if b.is_empty() {
        return Ok(None);
    }
    if b.len() < LIST_NAME_AT || b[STAT_BYTES + 1] != 0 {
        return Err(Error::Malformed);
    }
    // The name's whole length; the reply carries its first units.
    let name_len = usize::from(b[STAT_BYTES]);
    let here = name_len.min(LIST_NAME_UNITS);
    if name_len == 0 || name_len > NAME_UNITS || b.len() != LIST_NAME_AT + here * 2 {
        return Err(Error::Malformed);
    }
    let (info, times) = stat_reply(&b[..STAT_BYTES])?;
    if info.object <= after_object {
        return Err(Error::Malformed);
    }
    let mut entry = DirEntry {
        cookie: info.object,
        name: [0; NAME_UNITS],
        name_len,
        info,
        times,
    };
    for (out, bytes) in entry.name.iter_mut().zip(b[LIST_NAME_AT..].chunks_exact(2)) {
        *out = u16::from_le_bytes(bytes.try_into().unwrap());
    }
    if !valid_name(&entry.name[..here]) {
        return Err(Error::Malformed);
    }
    Ok(Some(entry))
}
/// No control characters and none of the characters Windows forbids.
fn valid_name(units: &[u16]) -> bool {
    !units
        .iter()
        .any(|u| *u < 32 || [34, 42, 47, 58, 60, 62, 63, 92, 124].contains(u))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_replies_reject_impossible_names_and_identity_but_preserve_utf16() {
        let mut bytes = [0; 60];
        let times = FileTimes {
            created: 5,
            modified: 6,
            accessed: 0,
        };
        bytes[..56].copy_from_slice(&door_service_protocol::files::encode_stat(
            FileInfo {
                attributes: 0,
                bytes: 3,
                object: 7,
                version: 1,
                directory: false,
            },
            times,
        ));
        bytes[56] = 1;
        for unit in [0, 1, 31, 34, 42, 47, 58, 60, 62, 63, 92, 124] {
            bytes[58..].copy_from_slice(&u16::to_le_bytes(unit));
            assert!(matches!(decode_entry(&bytes, 0), Err(Error::Malformed)));
        }
        for unit in [32, 46, 0xd800, 0xdfff, 0xffff] {
            bytes[58..].copy_from_slice(&u16::to_le_bytes(unit));
            let entry = decode_entry(&bytes, 0).unwrap().unwrap();
            assert_eq!(entry.name[0], unit);
            assert_eq!(entry.times, times);
            assert!(matches!(decode_entry(&bytes, 7), Err(Error::Malformed)));
        }
        // Out-of-range time and the old 32-byte layout are malformed.
        let mut bad = bytes;
        bad[55] = 0x80;
        assert!(matches!(decode_entry(&bad, 0), Err(Error::Malformed)));
        assert!(matches!(
            decode_entry(&bytes[24..], 0),
            Err(Error::Malformed)
        ));
        for offset in [8, 16] {
            let mut invalid = bytes;
            invalid[offset..offset + 8].fill(0);
            assert!(matches!(decode_entry(&invalid, 0), Err(Error::Malformed)));
        }
        assert_eq!(decode_entry(&[], 0).unwrap(), None);
    }
    #[test]
    fn long_names_arrive_as_a_first_part_with_the_whole_length() {
        use door_service_protocol::files::{LIST_NAME_AT, LIST_NAME_UNITS};
        let mut bytes = [0; 256];
        bytes[..56].copy_from_slice(&door_service_protocol::files::encode_stat(
            FileInfo {
                attributes: 0,
                bytes: 3,
                object: 9,
                version: 1,
                directory: false,
            },
            FileTimes::default(),
        ));
        bytes[56] = 200;
        for unit in bytes[LIST_NAME_AT..].chunks_exact_mut(2) {
            unit.copy_from_slice(&u16::to_le_bytes(u16::from(b'a')));
        }
        let first = LIST_NAME_AT + LIST_NAME_UNITS * 2;
        let entry = decode_entry(&bytes[..first], 0).unwrap().unwrap();
        assert_eq!(entry.name_len, 200);
        assert!(entry.name[..LIST_NAME_UNITS].iter().all(|u| *u == 97));
        assert!(entry.name[LIST_NAME_UNITS..].iter().all(|u| *u == 0));
        // A reply longer or shorter than the first part is malformed.
        assert!(decode_entry(&bytes[..first + 2], 0).is_err());
        assert!(decode_entry(&bytes[..first - 2], 0).is_err());
        // A short name still carries all of its units.
        bytes[56] = 5;
        assert!(decode_entry(&bytes[..LIST_NAME_AT + 10], 0).is_ok());
        assert!(decode_entry(&bytes[..first], 0).is_err());
    }
}
