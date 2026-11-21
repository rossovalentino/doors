//! Adversarial authenticated IPC bypasses SDK ownership checks intentionally.
use super::*;
use door_service_protocol::{Frame, Message, manifest, opcode};
pub(super) struct Raw {
    channel: u64,
    instance: u64,
    next: u64,
}
impl Raw {
    pub(super) fn new(registry: u64) -> Result<Self> {
        let mut init = door_service_client::Init::connect(registry)?;
        let port = init.resolve(manifest::STORAGE)?;
        let connected = door_sys::port_connect(port);
        let _ = door_sys::close(port);
        let channel = connected?;
        let identity = door_sys::identity(channel)?;
        check(
            identity.manifest == manifest::STORAGE && identity.instance != 0,
            "raw storage authority",
        )?;
        Ok(Self {
            channel,
            instance: identity.instance,
            next: 1,
        })
    }
    pub(super) fn call(&mut self, operation: u32, body: &[u8]) -> Result<(Code, Vec<u8>)> {
        let request = self.next;
        self.next += 1;
        let message = Message::new(operation, request, self.instance, Code::Success, body, 0)?;
        let deadline = door_sys::time()? + 100;
        door_sys::channel_send_deadline(self.channel, message.bytes(), &[], deadline)?;
        let mut bytes = [0; door_abi::MAX_PACKET];
        let mut transfers = [door_abi::Transfer::default(); door_abi::MAX_TRANSFERS];
        loop {
            match door_sys::channel_receive(self.channel, &mut bytes, &mut transfers) {
                Ok(meta) => {
                    for transfer in &transfers[..meta.handle_count as usize] {
                        let _ = door_sys::close(transfer.handle);
                    }
                    check(meta.handle_count == 0, "raw reply no handles")?;
                    let frame = Frame::decode(&bytes[..meta.packet_bytes as usize])?;
                    check(
                        frame.header.opcode == operation
                            && frame.header.request_id == request
                            && frame.instance == self.instance
                            && frame.header.handle_count == 0,
                        "raw exact reply identity",
                    )?;
                    return Ok((frame.code, frame.body.to_vec()));
                }
                Err(Status::WouldBlock) => {
                    check(door_sys::time()? < deadline, "raw storage deadline")?;
                    door_sys::yield_now()?;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    fn ok(&mut self, operation: u32, body: &[u8]) -> Result<Vec<u8>> {
        let (code, reply) = self.call(operation, body)?;
        check(code == Code::Success, "raw successful reply")?;
        Ok(reply)
    }
    fn open(&mut self, uid: u32) -> Result<u64> {
        let path = path(uid)?.as_bytes();
        let mut body = [0; 8 + 192];
        body[0] = Disposition::OpenExisting as u8;
        body[2..4].copy_from_slice(&(READ | WRITE).to_le_bytes());
        body[4..6].copy_from_slice(&SHARE_ALL.to_le_bytes());
        body[6..8].copy_from_slice(&(path.len() as u16).to_le_bytes());
        body[8..8 + path.len()].copy_from_slice(path);
        let reply = self.ok(opcode::OPEN, &body[..8 + path.len()])?;
        check(reply.len() == 8, "raw file token length")?;
        Ok(u64::from_le_bytes(reply[..].try_into().unwrap()))
    }
}
impl Drop for Raw {
    fn drop(&mut self) {
        let _ = door_sys::close(self.channel);
    }
}
pub fn exercise(registry: u64, uid: u32, original: door_service_client::FileInfo) -> Result<()> {
    let mut a = Raw::new(registry)?;
    let mut b = Raw::new(registry)?;
    let afile = a.open(uid)?;
    let bfile = b.open(uid)?;
    let begin = door_service_protocol::files::BeginReplace::new(
        afile,
        original.object,
        original.version,
        1,
    )
    .map_err(ClientError::Service)?;
    let reply = a.ok(opcode::REPLACE_BEGIN, &begin.encode())?;
    check(reply.len() == 8, "raw stage length")?;
    let token = u64::from_le_bytes(reply[..].try_into().unwrap());
    let mut body = [0; 25];
    body[..8].copy_from_slice(&afile.to_le_bytes());
    body[8..16].copy_from_slice(&token.to_le_bytes());
    body[24] = b'x';
    check(
        a.call(opcode::REPLACE_COMMIT, &body[..16])?.0 == Code::InvalidRequest,
        "server rejects incomplete commit",
    )?;
    a.ok(opcode::REPLACE_CHUNK, &body)?;
    for file in [afile, bfile] {
        body[..8].copy_from_slice(&file.to_le_bytes());
        for operation in [
            opcode::REPLACE_CHUNK,
            opcode::REPLACE_COMMIT,
            opcode::REPLACE_ABORT,
        ] {
            let n = if operation == opcode::REPLACE_CHUNK {
                25
            } else {
                16
            };
            let (code, reply) = b.call(operation, &body[..n])?;
            check(
                code == Code::InvalidHandle && reply.is_empty(),
                "server rejects foreign file and stage capabilities",
            )?;
        }
    }
    body[..8].copy_from_slice(&afile.to_le_bytes());
    check(
        a.call(opcode::REPLACE_CHUNK, &body)?.0 == Code::InvalidRequest,
        "server rejects repeated offset",
    )?;
    let actual = a.ok(opcode::FILE_INFO, &afile.to_le_bytes())?;
    check(
        door_service_client::FileInfo::decode(&actual).map_err(ClientError::Service)? == original,
        "foreign operations did not publish candidate",
    )?;
    a.ok(opcode::REPLACE_ABORT, &body[..16])?;
    a.ok(opcode::CLOSE, &afile.to_le_bytes())?;
    b.ok(opcode::CLOSE, &bfile.to_le_bytes())?;
    log(format_args!(
        "DOOR_P12 SDK REPLACE WIRE PASS uid={uid} foreign_file=1 foreign_stage=1 chunk=1 commit=1 abort=1 incomplete=1 offset=1\n"
    ));
    Ok(())
}
