//! Explicit private SDK exercise: retire authority, poll discovery, never replay.
use super::*;
use door_service_client::StorageConnect;

fn connected(registry: u64) -> Result<Storage> {
    let mut attempt = StorageConnect::begin(registry, door_sys::time()?)?;
    loop {
        match attempt.poll(door_sys::time()?)? {
            Some(storage) => {
                check(
                    matches!(
                        attempt.poll(door_sys::time()?),
                        Err(ClientError::Service(Code::Closing))
                    ),
                    "reconnect attempt consumed once",
                )?;
                return Ok(storage);
            }
            None => door_sys::sleep(door_sys::time()? + 1)?,
        }
    }
}
pub fn exercise(registry: u64, uid: u32) -> Result<()> {
    let mut storage = Storage::connect(registry)?;
    let baseline = storage.metadata(path(uid)?)?;
    for _ in 0..64 {
        let file = storage.open(
            path(uid)?,
            Disposition::OpenExisting,
            READ | WRITE,
            SHARE_ALL,
        )?;
        let key = storage.registry_open(
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &[68, 111, 111, 114],
            READ,
            false,
        )?;
        let mut staging = storage.begin_replace(&file, baseline, 3)?;
        storage.replace_chunk(&mut staging, b"x")?;
        storage.retire();
        storage.retire();
        check(storage.needs_reconnect(), "retired connection signal")?;
        check(
            matches!(
                storage.info(&file),
                Err(ClientError::Service(Code::InvalidHandle))
            ),
            "retired file rejected",
        )?;
        let mut fresh = connected(registry)?;
        check(!fresh.needs_reconnect(), "fresh authenticated connection")?;
        check(
            matches!(
                fresh.info(&file),
                Err(ClientError::Service(Code::InvalidHandle))
            ),
            "foreign file rejected",
        )?;
        check(
            matches!(
                fresh.replace_chunk(&mut staging, b"y"),
                Err(ClientError::Service(Code::InvalidHandle))
            ),
            "foreign staging rejected",
        )?;
        check(
            matches!(
                fresh.abort_replace(staging),
                Err(ClientError::Service(Code::InvalidHandle))
            ),
            "foreign staging abort rejected",
        )?;
        check(
            matches!(
                fresh.registry_close(key),
                Err(ClientError::Service(Code::InvalidHandle))
            ),
            "foreign registry key rejected",
        )?;
        check(
            matches!(
                fresh.close(file),
                Err(ClientError::Service(Code::InvalidHandle))
            ),
            "foreign close rejected",
        )?;
        // Exclusive sharing proves the old broker-side file grant was reclaimed.
        // Accepting a fresh channel can precede reclaiming an older slot in
        // the broker's fair scan. Only this read-only existing-open probe may
        // retry a definite sharing refusal; no mutation or uncertain result is replayed.
        let reclaim_deadline = door_sys::time()?.saturating_add(100);
        let reopened = loop {
            match fresh.open(path(uid)?, Disposition::OpenExisting, READ | WRITE, 0) {
                Ok(file) => break file,
                Err(ClientError::Service(Code::SharingViolation))
                    if door_sys::time()? < reclaim_deadline =>
                {
                    door_sys::sleep(door_sys::time()? + 1)?
                }
                Err(error) => return Err(error.into()),
            }
        };
        check(
            fresh.info(&reopened)? == baseline,
            "staged upload never published",
        )?;
        fill_half(&mut fresh, &reopened, uid, 0, 4096, false)?;
        fresh.close(reopened)?;
        storage = fresh;
    }
    storage.retire();
    log(format_args!(
        "DOOR_P12 STORAGE_CONNECT PASS uid={uid} cycles=64 stale_file=1 stale_key=1 stale_stage=1 unchanged=1\n"
    ));
    Ok(())
}
