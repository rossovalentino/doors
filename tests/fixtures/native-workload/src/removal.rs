//! Explicit SDK deletion exercise on the orchestrator's disposable disk only.
use super::*;
use door_service_protocol::{files::RemoveChild, opcode};
const HOME: &str = "/users/1001";
const FILE: &str = "/users/1001/remove.bin";
const FOLDER: &str = "/users/1001/remove-folder";
const CHILD: &str = "/users/1001/remove-folder/child.bin";
pub fn prepare(registry: u64, uid: u32) -> Result<()> {
    check(uid == 1001, "removal fixture identity")?;
    let mut s = Storage::connect(registry)?;
    let file = s.open(FILE, Disposition::CreateNew, READ | WRITE, SHARE_ALL)?;
    check(
        s.write(&file, 0, b"Removal binary\0\xff\n")? == 17,
        "removal fixture payload",
    )?;
    s.close(file)?;
    s.mkdir(FOLDER)?;
    let file = s.open(CHILD, Disposition::CreateNew, READ | WRITE, SHARE_ALL)?;
    check(
        s.write(&file, 0, b"Pending child\n")? == 14,
        "removal child payload",
    )?;
    s.close(file)?;
    s.flush()?;
    log(format_args!(
        "DOOR_P12 SDK REMOVE PREPARED uid={uid} file=1 folder=1 child=1\n"
    ));
    Ok(())
}
fn rejected(result: door_service_client::Result<()>, expected: Code) -> Result<()> {
    check(
        matches!(result, Err(ClientError::Service(code)) if code == expected),
        "removal exact rejection",
    )
}
pub fn exercise(registry: u64, uid: u32, console: &mut Console) -> Result<()> {
    check(uid == 1001, "removal fixture identity")?;
    let mut s = Storage::connect(registry)?;
    let parent = s.metadata(HOME)?.object;
    let file = s.open(FILE, Disposition::OpenExisting, READ | WRITE, SHARE_ALL)?;
    let original = s.info(&file)?;
    let mut raw = replacement_wire::Raw::new(registry)?;
    let encoded = RemoveChild::new(parent, original.object, original.version)
        .map_err(ClientError::Service)?
        .encode();
    for length in 0..24 {
        let (code, reply) = raw.call(opcode::REMOVE_CHILD_CHECKED, &encoded[..length])?;
        check(
            code == Code::InvalidRequest && reply.is_empty(),
            "remove truncated wire",
        )?;
    }
    let mut extra = encoded.to_vec();
    extra.push(0);
    check(
        raw.call(opcode::REMOVE_CHILD_CHECKED, &extra)?.0 == Code::InvalidRequest,
        "remove trailing wire",
    )?;
    for offset in [0, 8, 16] {
        let mut zero = encoded;
        zero[offset..offset + 8].fill(0);
        check(
            raw.call(opcode::REMOVE_CHILD_CHECKED, &zero)?.0 == Code::InvalidRequest,
            "remove zero wire",
        )?;
    }
    rejected(
        s.remove_child_checked(parent, original.object, original.version + 1),
        Code::Changed,
    )?;
    rejected(
        s.remove_child_checked(1, original.object, original.version),
        Code::Changed,
    )?;
    rejected(s.remove_child_checked(1, 2, 1), Code::Denied)?;
    // Provisioned foreign home18 remains private even through a new raw channel.
    let foreign = RemoveChild::new(18, original.object, original.version)
        .map_err(ClientError::Service)?
        .encode();
    check(
        raw.call(opcode::REMOVE_CHILD_CHECKED, &foreign)?.0 == Code::Denied,
        "remove foreign ancestor denied",
    )?;
    let mut peer = Storage::connect(registry)?;
    let denied = peer.open(FILE, Disposition::OpenExisting, READ, READ | WRITE)?;
    rejected(
        s.remove_child_checked(parent, original.object, original.version),
        Code::SharingViolation,
    )?;
    check(
        raw.call(opcode::REMOVE_CHILD_CHECKED, &encoded)?.0 == Code::SharingViolation,
        "raw peer sharing",
    )?;
    peer.close(denied)?;
    s.rename_child_checked(parent, original.object, original.version, &[0xd800, 46, 98])?;
    let changed = s.info(&file)?;
    s.rename_child_checked(
        parent,
        original.object,
        changed.version,
        &"remove.bin".encode_utf16().collect::<Vec<_>>(),
    )?;
    let current = s.info(&file)?;
    rejected(
        s.remove_child_checked(parent, original.object, original.version),
        Code::Changed,
    )?;
    let folder = s.metadata(FOLDER)?;
    let child = s.open(CHILD, Disposition::OpenExisting, READ, SHARE_ALL)?;
    let child_info = s.info(&child)?;
    rejected(
        s.remove_child_checked(parent, folder.object, folder.version),
        Code::InvalidRequest,
    )?;
    s.remove_child_checked(folder.object, child_info.object, child_info.version)?;
    check(s.list(FOLDER, 0)?.is_none(), "pending child hidden")?;
    rejected(
        s.remove_child_checked(parent, folder.object, folder.version),
        Code::InvalidRequest,
    )?;
    peer_read(&mut s, &child, b"Pending child\n")?;
    s.close(child)?;
    s.remove_child_checked(parent, folder.object, folder.version)?;
    s.remove_child_checked(parent, current.object, current.version)?;
    peer_read(&mut s, &file, b"Removal binary\0\xff\n")?;
    rejected(
        s.remove_child_checked(parent, current.object, current.version),
        Code::DeletePending,
    )?;
    check(
        matches!(
            s.metadata(FILE),
            Err(ClientError::Service(Code::DeletePending))
        ),
        "pending binding hidden",
    )?;
    log(format_args!(
        "DOOR_P12 SDK REMOVE PENDING uid={uid} object={} version={} bytes=17\n",
        current.object, current.version
    ));
    // The controller snapshots the acknowledged pending state before allowing
    // this live handle to close; a separate boot reclaims that actual image.
    loop {
        match console.read() {
            Ok(b'p') => break,
            Ok(_)
            | Err(ClientError::Service(Code::WouldBlock))
            | Err(ClientError::Transport(Status::WouldBlock)) => door_sys::yield_now()?,
            Err(error) => return Err(error.into()),
        }
    }
    s.close(file)?;
    s.flush()?;
    verify(registry, uid)?;
    log(format_args!(
        "DOOR_P12 SDK REMOVE PASS uid={uid} malformed=28 stale=1 foreign=1 protected=1 sharing=1 rename_utf16=1 pending=1 empty=1 nonempty=1\n"
    ));
    Ok(())
}
fn peer_read(s: &mut Storage, file: &door_service_client::File, expected: &[u8]) -> Result<()> {
    let mut bytes = [0; 32];
    let n = s.read(file, 0, &mut bytes)?;
    check(&bytes[..n] == expected, "pending exact stream bytes")
}
pub fn verify(registry: u64, uid: u32) -> Result<()> {
    check(uid == 1001, "removal fixture identity")?;
    let mut s = Storage::connect(registry)?;
    for path in [FILE, FOLDER, CHILD] {
        check(
            matches!(s.metadata(path), Err(ClientError::Service(Code::NotFound))),
            "removal survives reboot",
        )?;
    }
    let file = s.open(path(uid)?, Disposition::OpenExisting, READ, SHARE_ALL)?;
    fill_half(&mut s, &file, uid, 0, 4096, false)?;
    s.close(file)?;
    log(format_args!(
        "DOOR_P12 SDK REMOVE VERIFIED uid={uid} absent=3 neighbor=4096\n"
    ));
    Ok(())
}
