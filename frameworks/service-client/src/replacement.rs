use super::*;
use door_service_protocol::files::{
    BeginReplace, LARGE_REPLACE_BYTES, REPLACE_BYTES, STAGE_CHUNK_BYTES, STAGE_SECTION_BYTES,
    StageSection,
};

/// Owned staging capability, scoped to one live Storage connection and File.
/// Finish with commit/abort, or close the file/connection to reclaim staging.
#[derive(Debug)]
pub struct Replacement {
    owner: (Handle, u64),
    request: BeginReplace,
    token: u64,
    received: usize,
}
impl Storage {
    pub fn begin_replace(
        &mut self,
        file: &File,
        observed: FileInfo,
        bytes: usize,
    ) -> Result<Replacement> {
        self.operation(|storage| {
            storage.check_file(file)?;
            if observed.directory || observed.bytes > REPLACE_BYTES as u64 {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let request = BeginReplace::new(file.token, observed.object, observed.version, bytes)
                .map_err(Error::Service)?;
            let token = storage
                .connection
                .call(opcode::REPLACE_BEGIN, &request.encode(), &[])?
                .token()?;
            Ok(Replacement {
                owner: storage.connection.key(),
                request,
                token,
                received: 0,
            })
        })
    }
    fn replacement_body(&self, replacement: &Replacement) -> Result<[u8; 16]> {
        if replacement.owner != self.connection.key() {
            return Err(Error::Service(Code::InvalidHandle));
        }
        let mut body = [0; 16];
        body[..8].copy_from_slice(&replacement.request.file().to_le_bytes());
        body[8..].copy_from_slice(&replacement.token.to_le_bytes());
        Ok(body)
    }
    pub fn replace_chunk(&mut self, replacement: &mut Replacement, input: &[u8]) -> Result<()> {
        self.operation(|storage| {
            let header = storage.replacement_body(replacement)?;
            let end = replacement
                .received
                .checked_add(input.len())
                .ok_or(Error::Service(Code::InvalidRequest))?;
            if input.is_empty() || input.len() > CHUNK_BYTES || end > replacement.request.bytes() {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0; 24 + CHUNK_BYTES];
            body[..16].copy_from_slice(&header);
            body[16..24].copy_from_slice(&(replacement.received as u64).to_le_bytes());
            body[24..24 + input.len()].copy_from_slice(input);
            storage
                .connection
                .call(opcode::REPLACE_CHUNK, &body[..24 + input.len()], &[])?
                .empty()?;
            replacement.received = end;
            Ok(())
        })
    }
    pub fn abort_replace(&mut self, replacement: Replacement) -> Result<()> {
        self.operation(|storage| {
            let body = storage.replacement_body(&replacement)?;
            storage
                .connection
                .call(opcode::REPLACE_ABORT, &body, &[])?
                .empty()
        })
    }
    pub fn commit_replace(&mut self, replacement: Replacement) -> Result<FileInfo> {
        self.operation(|storage| {
            let body = storage.replacement_body(&replacement)?;
            if replacement.received != replacement.request.bytes() {
                let _ = storage.abort_replace(replacement);
                return Err(Error::Service(Code::InvalidRequest));
            }
            let reply = storage
                .connection
                .call(opcode::REPLACE_COMMIT, &body, &[])?;
            let info = FileInfo::decode(reply.data()?).map_err(|_| Error::Malformed)?;
            if info.directory
                || info.object != replacement.request.object()
                || Some(info.version) != replacement.request.version().checked_add(1)
                || info.bytes != replacement.request.bytes() as u64
            {
                return Err(Error::Malformed);
            }
            Ok(info)
        })
    }
    /// Success acknowledges one durable payload/length/version transaction.
    /// An uncertain transport failure retains caller bytes; never retry blindly.
    pub fn replace_checked(
        &mut self,
        file: &File,
        observed: FileInfo,
        input: &[u8],
    ) -> Result<FileInfo> {
        self.operation(|storage| {
            let replacement = storage.prepare_replace(file, observed, input)?;
            storage.commit_replace(replacement)
        })
    }
    /// Preparation cannot publish file bytes. Callers can distinguish a later
    /// uncertain COMMIT from a failed private upload without inspecting transport.
    pub fn prepare_replace(
        &mut self,
        file: &File,
        observed: FileInfo,
        input: &[u8],
    ) -> Result<Replacement> {
        self.operation(|storage| {
            let mut replacement = storage.begin_replace(file, observed, input.len())?;
            for chunk in input.chunks(CHUNK_BYTES) {
                if let Err(error) = storage.replace_chunk(&mut replacement, chunk) {
                    let _ = storage.abort_replace(replacement);
                    return Err(error);
                }
            }
            Ok(replacement)
        })
    }
}

/// Owned staged whole-file replacement (STAGE_*): up to the per-file cap,
/// scoped to one live Storage connection and File. Bytes upload privately;
/// commit publishes payload, length and version in one durable transaction.
/// Finish with commit/abort; closing the file or connection aborts it.
#[derive(Debug)]
pub struct LargeReplacement {
    owner: (Handle, u64),
    request: BeginReplace,
    token: u64,
    received: usize,
}
impl Storage {
    pub fn begin_large_replace(
        &mut self,
        file: &File,
        observed: FileInfo,
        bytes: usize,
    ) -> Result<LargeReplacement> {
        self.operation(|storage| {
            storage.check_file(file)?;
            if observed.directory || bytes > LARGE_REPLACE_BYTES {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let request = BeginReplace::large(file.token, observed.object, observed.version, bytes)
                .map_err(Error::Service)?;
            let token = storage
                .connection
                .call(opcode::STAGE_BEGIN, &request.encode(), &[])?
                .token()?;
            Ok(LargeReplacement {
                owner: storage.connection.key(),
                request,
                token,
                received: 0,
            })
        })
    }
    fn large_body(&self, r: &LargeReplacement) -> Result<[u8; 16]> {
        if r.owner != self.connection.key() {
            return Err(Error::Service(Code::InvalidHandle));
        }
        let mut body = [0; 16];
        body[..8].copy_from_slice(&r.request.file().to_le_bytes());
        body[8..].copy_from_slice(&r.token.to_le_bytes());
        Ok(body)
    }
    /// Append the next bytes in order (any length; sent in STAGE_CHUNK_BYTES
    /// requests, a whole message body each).
    pub fn large_replace_write(&mut self, r: &mut LargeReplacement, input: &[u8]) -> Result<()> {
        self.operation(|storage| {
            let header = storage.large_body(r)?;
            let end = r
                .received
                .checked_add(input.len())
                .ok_or(Error::Service(Code::InvalidRequest))?;
            if end > r.request.bytes() {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0; 24 + STAGE_CHUNK_BYTES];
            body[..16].copy_from_slice(&header);
            for chunk in input.chunks(STAGE_CHUNK_BYTES) {
                body[16..24].copy_from_slice(&(r.received as u64).to_le_bytes());
                body[24..24 + chunk.len()].copy_from_slice(chunk);
                storage
                    .connection
                    .call(opcode::STAGE_WRITE, &body[..24 + chunk.len()], &[])?
                    .empty()?;
                r.received += chunk.len();
            }
            Ok(())
        })
    }
    /// Append the next `bytes` candidate bytes from `section` (filled by the
    /// caller from `section_offset`), one STAGE_WRITE_SECTION request per
    /// `STAGE_SECTION_BYTES`: the service snapshots them, so a 4 MiB save is
    /// 64 requests instead of ~19,600 messages. The caller keeps the handle.
    pub fn large_replace_write_section(
        &mut self,
        r: &mut LargeReplacement,
        section: Handle,
        section_offset: u64,
        bytes: usize,
    ) -> Result<()> {
        self.operation(|storage| {
            storage.large_body(r)?;
            let end = r
                .received
                .checked_add(bytes)
                .filter(|end| *end <= r.request.bytes())
                .ok_or(Error::Service(Code::InvalidRequest))?;
            let mut done = 0;
            while r.received < end {
                let n = (end - r.received).min(STAGE_SECTION_BYTES);
                let at = section_offset
                    .checked_add(done as u64)
                    .ok_or(Error::Service(Code::InvalidRequest))?;
                let body = StageSection::new(r.request.file(), r.token, r.received as u64, at, n)
                    .map_err(Error::Service)?
                    .encode();
                storage
                    .connection
                    .call(
                        opcode::STAGE_WRITE_SECTION,
                        &body,
                        // The kernel snapshot leases the section like a
                        // mapping: it requires READ and MAP on the grant.
                        &[Transfer {
                            handle: section,
                            rights: rights::QUERY | rights::READ | rights::MAP | rights::TRANSFER,
                            mode: 0,
                        }],
                    )?
                    .empty()?;
                r.received += n;
                done += n;
            }
            Ok(())
        })
    }
    /// As `prepare_large_replace`, with the candidate already in `section`
    /// from offset 0.
    pub fn prepare_large_replace_section(
        &mut self,
        file: &File,
        observed: FileInfo,
        section: Handle,
        bytes: usize,
    ) -> Result<LargeReplacement> {
        self.operation(|storage| {
            let mut r = storage.begin_large_replace(file, observed, bytes)?;
            if let Err(error) = storage.large_replace_write_section(&mut r, section, 0, bytes) {
                let _ = storage.abort_large_replace(r);
                return Err(error);
            }
            Ok(r)
        })
    }
    pub fn abort_large_replace(&mut self, r: LargeReplacement) -> Result<()> {
        self.operation(|storage| {
            let body = storage.large_body(&r)?;
            storage
                .connection
                .call(opcode::STAGE_ABORT, &body, &[])?
                .empty()
        })
    }
    /// Success acknowledges one durable payload/length/version transaction;
    /// the attempt consumes the candidate either way. An uncertain transport
    /// failure may still have committed: never retry blindly.
    pub fn commit_large_replace(&mut self, r: LargeReplacement) -> Result<FileInfo> {
        self.operation(|storage| {
            let body = storage.large_body(&r)?;
            if r.received != r.request.bytes() {
                let _ = storage.abort_large_replace(r);
                return Err(Error::Service(Code::InvalidRequest));
            }
            let reply = storage.connection.call(opcode::STAGE_COMMIT, &body, &[])?;
            let info = FileInfo::decode(reply.data()?).map_err(|_| Error::Malformed)?;
            if info.directory
                || info.object != r.request.object()
                || Some(info.version) != r.request.version().checked_add(1)
                || info.bytes != r.request.bytes() as u64
            {
                return Err(Error::Malformed);
            }
            Ok(info)
        })
    }
    /// Upload a complete candidate privately; a failure aborts it and
    /// publishes nothing.
    pub fn prepare_large_replace(
        &mut self,
        file: &File,
        observed: FileInfo,
        input: &[u8],
    ) -> Result<LargeReplacement> {
        self.operation(|storage| {
            let mut r = storage.begin_large_replace(file, observed, input.len())?;
            if let Err(error) = storage.large_replace_write(&mut r, input) {
                let _ = storage.abort_large_replace(r);
                return Err(error);
            }
            Ok(r)
        })
    }
}
