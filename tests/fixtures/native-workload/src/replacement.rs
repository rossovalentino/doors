//! Explicit private SDK exercise; ordinary save/reopen contracts stay unchanged.
use super::*;
pub const SAVED: &[u8] = b"Checked atomic document\0\xff\n";
fn bytes(storage: &mut Storage, file: &door_service_client::File, expected: &[u8]) -> Result<()> {
    check(
        storage.info(file)?.bytes == expected.len() as u64,
        "replacement exact length",
    )?;
    let mut out = [0; 192];
    for offset in (0..expected.len()).step_by(192) {
        let n = storage.read(file, offset as u64, &mut out)?;
        let len = (expected.len() - offset).min(out.len());
        check(
            n == len && out[..n] == expected[offset..offset + len],
            "replacement exact bytes",
        )?;
    }
    check(
        storage.read(file, expected.len() as u64, &mut out)? == 0,
        "replacement exact EOF",
    )
}
fn pattern_bytes(uid: u32) -> Vec<u8> {
    (0..4096).map(|i| pattern(i, uid)).collect()
}
pub fn exercise(registry: u64, uid: u32) -> Result<()> {
    let mut storage = Storage::connect(registry)?;
    let file = storage.open(
        path(uid)?,
        Disposition::OpenExisting,
        READ | WRITE,
        SHARE_ALL,
    )?;
    let mut other = Storage::connect(registry)?;
    let peer = other.open(
        path(uid)?,
        Disposition::OpenExisting,
        READ | WRITE,
        SHARE_ALL,
    )?;
    let original = storage.info(&file)?;
    let old = pattern_bytes(uid);
    bytes(&mut other, &peer, &old)?;
    let mut candidate = storage.begin_replace(&file, original, 3)?;
    check(
        matches!(
            storage.begin_replace(&file, original, 3),
            Err(ClientError::Service(Code::WouldBlock))
        ),
        "replacement duplicate stage",
    )?;
    storage.replace_chunk(&mut candidate, b"n")?;
    check(
        matches!(
            other.replace_chunk(&mut candidate, b"o"),
            Err(ClientError::Service(Code::InvalidHandle))
        ),
        "replacement foreign SDK connection",
    )?;
    storage.flush()?;
    check(
        other.info(&peer)? == original,
        "private stage unchanged metadata after unrelated flush",
    )?;
    bytes(&mut other, &peer, &old)?;
    storage.abort_replace(candidate)?;
    replacement_wire::exercise(registry, uid, original)?;
    let reader = storage.open(path(uid)?, Disposition::OpenExisting, READ, SHARE_ALL)?;
    check(
        matches!(
            storage.begin_replace(&reader, original, 0),
            Err(ClientError::Service(Code::Denied))
        ),
        "replacement write grant",
    )?;
    storage.close(reader)?;
    check(
        matches!(
            storage.begin_replace(
                &file,
                door_service_client::FileInfo {
                    version: original.version + 1,
                    ..original
                },
                0
            ),
            Err(ClientError::Service(Code::Changed))
        ),
        "replacement stale begin",
    )?;
    check(
        matches!(
            storage.begin_replace(
                &file,
                door_service_client::FileInfo {
                    object: original.object + 1,
                    ..original
                },
                0
            ),
            Err(ClientError::Service(Code::Changed))
        ),
        "replacement wrong object",
    )?;
    // Four private candidates occupy bounded static slots. Close must reclaim
    // them without publishing any candidate bytes, even with live SDK values.
    let mut held = Vec::new();
    for _ in 0..4 {
        let f = storage.open(
            path(uid)?,
            Disposition::OpenExisting,
            READ | WRITE,
            SHARE_ALL,
        )?;
        let stage = storage.begin_replace(&f, original, 4096)?;
        held.push((f, stage));
    }
    check(
        matches!(
            storage.begin_replace(&file, original, 1),
            Err(ClientError::Service(Code::Resources))
        ),
        "replacement bounded capacity",
    )?;
    for (f, _) in held {
        storage.close(f)?;
    }
    let stage = storage.begin_replace(&file, original, 0)?;
    storage.abort_replace(stage)?;
    // A real second connection changes the comparison version after Begin.
    let mut stale = storage.begin_replace(&file, original, 3)?;
    storage.replace_chunk(&mut stale, b"bad")?;
    check(other.write(&peer, 0, &old[..1])? == 1, "competing write")?;
    other.flush()?;
    check(
        matches!(
            storage.commit_replace(stale),
            Err(ClientError::Service(Code::Changed))
        ),
        "replacement competing write rejected",
    )?;
    bytes(&mut other, &peer, &old)?;
    let changed = storage.info(&file)?;
    check(
        changed.version == original.version + 1,
        "only competing write advanced version",
    )?;
    let max: Vec<u8> = (0..4096).map(|i| pattern(i, uid) ^ 0xff).collect();
    let full = storage.replace_checked(&file, changed, &max)?;
    bytes(&mut other, &peer, &max)?;
    let empty = storage.replace_checked(&file, full, &[])?;
    bytes(&mut other, &peer, &[])?;
    let saved = storage.replace_checked(&file, empty, SAVED)?;
    bytes(&mut other, &peer, SAVED)?;
    check(
        saved.object == original.object && saved.version == original.version + 4,
        "compound version and identity",
    )?;
    other.close(peer)?;
    storage.close(file)?;
    log(format_args!(
        "DOOR_P12 SDK REPLACE PASS uid={uid} versions=4 private=1 stale=1 grant=1 capacity=1 close=1 max=1 empty=1\n"
    ));
    Ok(())
}
pub fn restore(registry: u64, uid: u32) -> Result<()> {
    let mut storage = Storage::connect(registry)?;
    let file = storage.open(
        path(uid)?,
        Disposition::OpenExisting,
        READ | WRITE,
        SHARE_ALL,
    )?;
    let before = storage.info(&file)?;
    bytes(&mut storage, &file, SAVED)?;
    let restored = storage.replace_checked(&file, before, &pattern_bytes(uid))?;
    check(
        restored.object == before.object && restored.version == before.version + 1,
        "restore same identity and one version",
    )?;
    fill_half(&mut storage, &file, uid, 0, 4096, false)?;
    storage.close(file)?;
    log(format_args!(
        "DOOR_P12 SDK RESTORE PASS uid={uid} bytes=4096 versions=1 identity=1\n"
    ));
    Ok(())
}
