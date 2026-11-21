//! P07 end-to-end transactions on the private VirtIO image. Interruptions occur
//! at production journal boundaries; fresh processes perform recovery.
use crate::block_device::Session;
use core::{
    cell::UnsafeCell,
    fmt::Write,
    sync::atomic::{AtomicBool, Ordering},
};
use door_boot_debug::Serial;
use door_boot_protocol::BootInfo;
use door_kernel::{
    persistence::{
        Access, Ace, Acl, CATALOG_BYTES, Context, Disposition, Error, OpenOptions, Path,
        PersistentVolume, Principal, RegistryRoot, RegistryType, RegistryView, Setting, State,
        Subject, Value,
    },
    storage::{
        BlockDevice, Checkpoint, Ext2, FileId, Gpt, Journal, Partition, RecoveryReport, Vfs,
        WorkingSet, crc32,
    },
};

struct Scratch {
    busy: AtomicBool,
    work: UnsafeCell<WorkingSet>,
    state: UnsafeCell<State>,
    catalog: UnsafeCell<[u8; CATALOG_BYTES]>,
}
// The BSP diagnostic owner holds `busy` for the complete borrow. Other CPUs run
// only their already-qualified idle/IPI path and never access this storage.
unsafe impl Sync for Scratch {}
static SCRATCH: Scratch = Scratch {
    busy: AtomicBool::new(false),
    work: UnsafeCell::new(WorkingSet::new()),
    state: UnsafeCell::new(State::new()),
    catalog: UnsafeCell::new([0; CATALOG_BYTES]),
};
type Volume<'a, 'b> = PersistentVolume<'a, Ext2<Journal<'a, &'b mut Session>>>;
const OLD_PATH: &str = "/users/1001/note-Ω.txt";
const NEW_PATH: &str = "/users/1001/revised-Ω.txt";
const KEEP_PATH: &str = "/users/1001/keep.bin";
const KEEP: &[u8] = b"Door P07 unrelated\n";
const REGISTRY_PATH: &[u16] = &[
    83, 111, 102, 116, 119, 97, 114, 101, 92, 68, 111, 111, 114, 92, 70, 105, 120, 116, 117, 114,
    101,
];
const EPOCH: &[u16] = &[69, 112, 111, 99, 104];
const BYTES: &[u16] = &[66, 121, 116, 101, 115];

pub fn is_mode(mode: &[u8]) -> bool {
    matches!(
        mode,
        b"persist-save"
            | b"persist-reopen"
            | b"journal-prepare"
            | b"journal-commit"
            | b"journal-first-home"
            | b"journal-home"
            | b"journal-checkpoint"
            | b"recover-old"
            | b"recover-new"
            | b"persist-old"
            | b"persist-new"
            | b"persist-full"
    )
}
fn subject(id: u32) -> Subject {
    // This fixture is trusted boot policy; future user requests must use the
    // executive's authenticated effective identity, never these literal claims.
    unsafe { Subject::authenticated(id, 0, if id == 1 { 2 } else { 1 }) }.unwrap()
}
fn options(access: Access, disposition: Disposition) -> OpenOptions {
    OpenOptions {
        access,
        share: Access::SHARE_ALL,
        disposition,
    }
}
fn content(buffer: &mut [u8; 6000], new: bool) {
    for (i, byte) in buffer.iter_mut().enumerate() {
        *byte = if new {
            ((i * 19 + 23) % 251) as u8
        } else {
            ((i * 37 + 11) % 251) as u8
        };
    }
}
fn file_acl(new: bool) -> Acl {
    let mut acl = Acl::owner_full(1001);
    acl.push(Ace {
        principal: Principal::User(1002),
        deny: true,
        access: if new {
            Access::READ.union(Access::WRITE)
        } else {
            Access::WRITE
        },
        inherit: false,
    })
    .unwrap();
    acl.push(Ace {
        principal: Principal::User(1002),
        deny: false,
        access: Access::READ,
        inherit: false,
    })
    .unwrap();
    acl
}
fn user_directory<F: Vfs>(volume: &mut PersistentVolume<'_, F>) {
    let mut acl = Acl::owner_full(1001);
    acl.push(Ace {
        principal: Principal::Everyone,
        deny: false,
        access: Access::TRAVERSE,
        inherit: true,
    })
    .unwrap();
    volume
        .mkdir(
            subject(1),
            Path::Native("/users/1001"),
            Context::root(),
            Some(acl),
        )
        .unwrap();
}
fn configure<F: Vfs>(volume: &mut PersistentVolume<'_, F>, new: bool) {
    let owner = subject(1001);
    volume
        .setting_set(owner, 1001, Setting::Theme, if new { 2 } else { 1 })
        .unwrap();
    volume
        .setting_set(
            owner,
            1001,
            Setting::ScalePercent,
            if new { 150 } else { 125 },
        )
        .unwrap();
    let key = volume
        .registry_open(
            owner,
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            REGISTRY_PATH,
            Access::READ.union(Access::WRITE),
            true,
        )
        .unwrap();
    let before = volume.registry_version(&key).unwrap();
    volume
        .registry_set(&key, EPOCH, Value::dword(if new { 2 } else { 1 }))
        .unwrap();
    volume
        .registry_set(
            &key,
            BYTES,
            Value::new(
                RegistryType::Binary,
                &[0xd0, 7, if new { 2 } else { 1 }, 0xa5],
            )
            .unwrap(),
        )
        .unwrap();
    assert!(volume.registry_changed(&key, before).unwrap());
    assert_eq!(
        volume.registry_enumerate_value(&key, 0, before),
        Err(Error::Changed)
    );
    volume.registry_close(key).unwrap();
}
#[inline(never)]
fn save(volume: &mut Volume<'_, '_>) -> FileId {
    user_directory(volume);
    let owner = subject(1001);
    let (file, created) = volume
        .open(
            owner,
            Path::Native(OLD_PATH),
            Context::root(),
            options(Access::ALL, Disposition::CreateNew),
        )
        .unwrap();
    assert!(created);
    let mut bytes = [0; 6000];
    content(&mut bytes, false);
    assert_eq!(volume.write(&file, 0, &bytes).unwrap(), bytes.len());
    volume.set_acl(owner, &file, file_acl(false)).unwrap();
    volume.set_attributes(&file, 0x20).unwrap();
    volume.close(file).unwrap();
    let (keep, _) = volume
        .open(
            owner,
            Path::Native(KEEP_PATH),
            Context::root(),
            options(Access::READ.union(Access::WRITE), Disposition::CreateNew),
        )
        .unwrap();
    assert_eq!(volume.write(&keep, 0, KEEP).unwrap(), KEEP.len());
    volume.close(keep).unwrap();
    let retired = lifetime(volume);
    configure(volume, false);
    registry_lifetime(volume);
    volume.commit().unwrap();
    retired
}

#[inline(never)]
fn registry_lifetime<F: Vfs>(volume: &mut PersistentVolume<'_, F>) {
    let owner = subject(1001);
    let parent = volume
        .registry_open(
            owner,
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &[82, 101, 116, 105, 114, 101, 100],
            Access::READ.union(Access::DELETE),
            true,
        )
        .unwrap();
    let child = volume
        .registry_open(
            owner,
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &[82, 101, 116, 105, 114, 101, 100, 92, 67, 104, 105, 108, 100],
            Access::READ.union(Access::DELETE),
            true,
        )
        .unwrap();
    volume.registry_delete_key(&child).unwrap();
    volume.registry_delete_key(&parent).unwrap();
    // Serialize the live tombstone chain through the production filesystem, then
    // close its parent first. Neither accepted update may make the catalog invalid.
    volume.flush().unwrap();
    volume.registry_close(parent).unwrap();
    volume.flush().unwrap();
    assert_eq!(volume.registry_version(&child), Err(Error::KeyDeleted));
    volume.registry_close(child).unwrap();
    assert_eq!(volume.setting_get(owner, 1001, Setting::Theme), Ok(1));
}

#[inline(never)]
fn lifetime<F: Vfs>(volume: &mut PersistentVolume<'_, F>) -> FileId {
    let owner = subject(1001);
    let left_path = "/users/1001/left.tmp";
    let right_path = "/users/1001/right.tmp";
    let (left, _) = volume
        .open(
            owner,
            Path::Native(left_path),
            Context::root(),
            options(Access::ALL, Disposition::CreateNew),
        )
        .unwrap();
    let (right, _) = volume
        .open(
            owner,
            Path::Native(right_path),
            Context::root(),
            options(Access::ALL, Disposition::CreateNew),
        )
        .unwrap();
    volume.write(&left, 0, b"left").unwrap();
    volume.write(&right, 0, b"prior").unwrap();
    let source = volume.info(&left).unwrap().backing;
    let replaced = volume.info(&right).unwrap().backing;
    volume
        .rename(
            owner,
            &left,
            Path::Native(right_path),
            Context::root(),
            true,
        )
        .unwrap();
    assert_eq!(volume.info(&left).unwrap().backing, source);
    let mut bytes = [0; 5];
    assert_eq!(volume.read(&right, 0, &mut bytes).unwrap(), 5);
    assert_eq!(&bytes, b"prior");
    assert!(volume.info(&right).unwrap().delete_pending);
    volume.close(right).unwrap();
    let (replacement, _) = volume
        .open(
            owner,
            Path::Native(right_path),
            Context::root(),
            options(Access::ALL, Disposition::TruncateExisting),
        )
        .unwrap();
    assert_eq!(volume.info(&replacement).unwrap().backing, source);
    assert_eq!(volume.info(&left).unwrap().bytes, 0);
    volume.write(&replacement, 0, b"again").unwrap();
    assert_eq!(volume.read(&left, 0, &mut bytes).unwrap(), 5);
    assert_eq!(&bytes, b"again");
    volume.delete(&replacement).unwrap();
    assert!(matches!(
        volume.open(
            owner,
            Path::Native(right_path),
            Context::root(),
            options(Access::READ, Disposition::OpenExisting)
        ),
        Err(Error::DeletePending)
    ));
    volume.close(left).unwrap();
    volume.close(replacement).unwrap();
    let (reused, _) = volume
        .open(
            owner,
            Path::Native(left_path),
            Context::root(),
            options(Access::ALL, Disposition::CreateNew),
        )
        .unwrap();
    assert_ne!(volume.info(&reused).unwrap().backing, replaced);
    assert_ne!(volume.info(&reused).unwrap().backing, source);
    volume.delete(&reused).unwrap();
    volume.close(reused).unwrap();
    replaced
}
#[inline(never)]
fn mutate(volume: &mut Volume<'_, '_>) {
    let owner = subject(1001);
    let (file, _) = volume
        .open(
            owner,
            Path::Native(OLD_PATH),
            Context::root(),
            options(Access::ALL, Disposition::OpenExisting),
        )
        .unwrap();
    let identity = volume.info(&file).unwrap().backing;
    let mut bytes = [0; 6000];
    content(&mut bytes, true);
    volume.write(&file, 0, &bytes).unwrap();
    volume
        .rename(owner, &file, Path::Native(NEW_PATH), Context::root(), false)
        .unwrap();
    assert_eq!(volume.info(&file).unwrap().backing, identity);
    volume.set_acl(owner, &file, file_acl(true)).unwrap();
    volume.close(file).unwrap();
    configure(volume, true);
}
#[inline(never)]
fn validate<F: Vfs>(volume: &mut PersistentVolume<'_, F>, new: bool) {
    let owner = subject(1001);
    let other = subject(1002);
    let path = if new { NEW_PATH } else { OLD_PATH };
    let (file, created) = volume
        .open(
            owner,
            Path::Native(path),
            Context::root(),
            options(Access::ALL, Disposition::OpenExisting),
        )
        .unwrap();
    assert!(!created);
    let identity = volume.info(&file).unwrap();
    assert_eq!(identity.bytes, 6000);
    assert_eq!(identity.attributes, 0x20);
    assert_eq!(volume.acl(&file).unwrap(), file_acl(new));
    let mut expected = [0; 6000];
    content(&mut expected, new);
    let mut actual = [0; 6000];
    assert_eq!(volume.read(&file, 0, &mut actual).unwrap(), actual.len());
    assert_eq!(actual, expected);
    assert!(matches!(
        volume.open(
            owner,
            Path::Native(path),
            Context::root(),
            OpenOptions {
                access: Access::READ,
                share: Access(0),
                disposition: Disposition::OpenExisting
            }
        ),
        Err(Error::SharingViolation)
    ));
    assert!(matches!(
        volume.open(
            other,
            Path::Native(path),
            Context::root(),
            options(Access::WRITE, Disposition::OpenExisting)
        ),
        Err(Error::AccessDenied)
    ));
    let other_read = volume.open(
        other,
        Path::Native(path),
        Context::root(),
        options(Access::READ, Disposition::OpenExisting),
    );
    if new {
        assert!(matches!(other_read, Err(Error::AccessDenied)));
    } else {
        let (read, _) = other_read.unwrap();
        assert_eq!(volume.info(&read).unwrap().backing, identity.backing);
        volume.close(read).unwrap();
    }
    // The Windows view resolves to the same object, including non-ASCII units.
    let mut windows = [0; 64];
    let name = if new {
        "C:\\Users\\1001\\revised-Ω.txt"
    } else {
        "C:\\Users\\1001\\note-Ω.txt"
    };
    let count = name.encode_utf16().count();
    for (out, unit) in windows.iter_mut().zip(name.encode_utf16()) {
        *out = unit;
    }
    let (alias, _) = volume
        .open(
            owner,
            Path::Windows(&windows[..count]),
            Context::root(),
            options(Access::READ, Disposition::OpenExisting),
        )
        .unwrap();
    assert_eq!(volume.info(&alias).unwrap().backing, identity.backing);
    volume.close(alias).unwrap();
    let mut network = [0; 64];
    network[2..count - 1].copy_from_slice(&windows[3..count]);
    for first in [47, 92] {
        for second in [47, 92] {
            network[0] = first;
            network[1] = second;
            assert!(matches!(
                volume.open(
                    owner,
                    Path::Windows(&network[..count - 1]),
                    Context::root(),
                    options(Access::READ, Disposition::OpenExisting),
                ),
                Err(Error::Unsupported)
            ));
        }
    }
    volume.close(file).unwrap();
    assert!(matches!(
        volume.open(
            owner,
            Path::Native(if new { OLD_PATH } else { NEW_PATH }),
            Context::root(),
            options(Access::READ, Disposition::OpenExisting)
        ),
        Err(Error::NotFound)
    ));
    let (keep, _) = volume
        .open(
            owner,
            Path::Native(KEEP_PATH),
            Context::root(),
            options(Access::READ, Disposition::OpenExisting),
        )
        .unwrap();
    let mut preserved = [0; 32];
    assert_eq!(volume.read(&keep, 0, &mut preserved).unwrap(), KEEP.len());
    assert_eq!(&preserved[..KEEP.len()], KEEP);
    volume.close(keep).unwrap();
    assert_eq!(
        volume.setting_get(owner, 1001, Setting::Theme).unwrap(),
        if new { 2 } else { 1 }
    );
    assert_eq!(
        volume
            .setting_get(owner, 1001, Setting::ScalePercent)
            .unwrap(),
        if new { 150 } else { 125 }
    );
    assert_eq!(
        volume.setting_get(other, 1001, Setting::Theme),
        Err(Error::AccessDenied)
    );
    assert_eq!(
        volume.setting_set(owner, 1001, Setting::ScalePercent, 126),
        Err(Error::InvalidParameter)
    );
    let key = volume
        .registry_open(
            owner,
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            REGISTRY_PATH,
            Access::READ,
            false,
        )
        .unwrap();
    assert_eq!(
        volume.registry_get(&key, EPOCH).unwrap(),
        Value::dword(if new { 2 } else { 1 })
    );
    assert_eq!(
        volume.registry_get(&key, BYTES).unwrap().data(),
        &[0xd0, 7, if new { 2 } else { 1 }, 0xa5]
    );
    assert_eq!(
        volume.registry_set(&key, EPOCH, Value::dword(9)),
        Err(Error::AccessDenied)
    );
    let version = volume.registry_version(&key).unwrap();
    assert!(volume.registry_enumerate_value(&key, 0, version).is_ok());
    assert_eq!(
        Value::new(RegistryType::Dword, &[1, 2]),
        Err(Error::TypeMismatch)
    );
    volume.registry_close(key).unwrap();
    assert!(matches!(
        volume.registry_open(
            other,
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            REGISTRY_PATH,
            Access::READ,
            false
        ),
        Err(Error::NotFound)
    ));
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_P07 PERSIST PASS state={} bytes=6000 crc32={:08x}",
        if new { "new" } else { "old" },
        crc32(&actual)
    )
    .ok();
    writeln!(
        serial,
        "DOOR_P07 SECURITY PASS acl=1 sharing=1 rename=1 metadata=1"
    )
    .ok();
    writeln!(
        serial,
        "DOOR_P07 CONFIG PASS native=1 registry=1 typed=1 unrelated=1"
    )
    .ok();
}
fn checkpoint(mode: &[u8]) -> Option<(Checkpoint, &'static str)> {
    match mode {
        b"journal-prepare" => Some((Checkpoint::PrepareFlushed, "prepare")),
        b"journal-commit" => Some((Checkpoint::CommitFlushed, "commit")),
        b"journal-first-home" => Some((Checkpoint::FirstHomeFlushed, "first_home")),
        b"journal-home" => Some((Checkpoint::HomeFlushed, "home")),
        b"journal-checkpoint" => Some((Checkpoint::CheckpointFlushed, "checkpoint")),
        _ => None,
    }
}
fn interrupted(stage: &str, generation: u64) -> ! {
    crate::arch::disable_interrupts();
    let (token, count) = crate::token();
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_P07 INTERRUPT stage={stage} generation={generation}"
    )
    .ok();
    writeln!(
        serial,
        "DOOR_TEST v1 {} JOURNAL_INTERRUPTED",
        core::str::from_utf8(&token[..count]).unwrap()
    )
    .ok();
    door_boot_debug::exit(18)
}
fn report_recovery(report: RecoveryReport, new: bool) {
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_P07 RECOVERY PASS state={} replayed={} generation={}",
        if new { "new" } else { "old" },
        u8::from(report.replayed),
        report.generation
    )
    .ok();
}

#[inline(never)]
pub fn verify(info: &BootInfo<'_>, mode: &[u8]) {
    assert!(is_mode(mode));
    assert!(!SCRATCH.busy.swap(true, Ordering::Acquire));
    let (work, state, catalog) = unsafe {
        (
            &mut *SCRATCH.work.get(),
            &mut *SCRATCH.state.get(),
            &mut *SCRATCH.catalog.get(),
        )
    };
    let mut disk = Session::open(info);
    let gpt = Gpt::read(&mut disk).unwrap();
    assert_eq!(gpt.disk_id, *b"DoorP07DiskGUID0");
    assert_eq!(gpt.partitions().count(), 3);
    let home = gpt
        .partitions()
        .find(|p| p.id == *b"DoorP07ExtGUID00")
        .unwrap();
    let log = gpt
        .partitions()
        .find(|p| p.id == *b"DoorP07LogGUID00")
        .unwrap();
    if mode == b"persist-full" {
        capacity(&mut disk, home, log, work, state, catalog);
    } else {
        let (mut journal, recovered) =
            Journal::open(&mut disk, home, log, gpt.disk_id, work).unwrap();
        journal.begin().unwrap();
        let fs = Ext2::mount(journal).unwrap();
        let mut volume = if mode == b"persist-save" {
            PersistentVolume::initialize(fs, state, catalog, subject(1)).unwrap()
        } else {
            PersistentVolume::mount(fs, state, catalog).unwrap()
        };
        let retired = if mode == b"persist-save" {
            Some(save(&mut volume))
        } else {
            None
        };
        if let Some((selected, stage)) = checkpoint(mode) {
            mutate(&mut volume);
            volume
                .commit_with(|observed| {
                    if observed == selected {
                        interrupted(stage, recovered.generation + 1);
                    }
                })
                .unwrap();
            panic!("journal interruption checkpoint was not reached");
        }
        let new = matches!(mode, b"recover-new" | b"persist-new");
        validate(&mut volume, new);
        // Successful save is generation1; read-only subsequent mounts publish no
        // extra transaction. Recovery provenance reports the state before save.
        report_recovery(recovered, new);
        let mut fs = volume.into_inner().unwrap();
        if let Some(retired) = retired {
            assert_eq!(
                fs.metadata(retired),
                Err(door_kernel::storage::Error::NotFound)
            );
            let mut serial = unsafe { Serial::new() };
            writeln!(
                serial,
                "DOOR_P07 LIFETIME PASS replace=1 delete=1 truncate=1 stale=1"
            )
            .ok();
        }
        let mut journal = fs.into_inner().unwrap();
        journal.abort().unwrap();
    }
    let statistics = disk.statistics();
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_P07 STACK PASS sampled_remaining={} stack_bytes={}",
        statistics.sampled_remaining, statistics.stack_bytes
    )
    .ok();
    disk.close();
    SCRATCH.busy.store(false, Ordering::Release);
    writeln!(
        serial,
        "DOOR_P07 CLEANUP PASS dma_quiesced=1 pages_reclaimed=8"
    )
    .ok();
}

#[inline(never)]
fn capacity(
    disk: &mut Session,
    home: Partition,
    log: Partition,
    work: &mut WorkingSet,
    state: &mut State,
    catalog: &mut [u8; CATALOG_BYTES],
) {
    let committed;
    {
        let (mut journal, _) =
            Journal::open(&mut *disk, home, log, *b"DoorP07DiskGUID0", work).unwrap();
        journal.begin().unwrap();
        let mut volume =
            PersistentVolume::initialize(Ext2::mount(journal).unwrap(), state, catalog, subject(1))
                .unwrap();
        user_directory(&mut volume);
        volume.commit().unwrap();
        let mut count = 0;
        loop {
            let mut name = *b"/users/1001/full00";
            name[16] = b'0' + count / 10;
            name[17] = b'0' + count % 10;
            let (file, _) = volume
                .open(
                    subject(1001),
                    Path::Native(core::str::from_utf8(&name).unwrap()),
                    Context::root(),
                    options(Access::READ.union(Access::WRITE), Disposition::CreateNew),
                )
                .unwrap();
            let data = [0x40 + count; 8192];
            match volume.write(&file, 0, &data) {
                Ok(8192) => {
                    volume.close(file).unwrap();
                    volume.commit().unwrap();
                    count += 1;
                    assert!(count < 20);
                }
                Err(Error::Storage(door_kernel::storage::Error::NoSpace)) => {
                    // Ext2 rejected allocator exhaustion before mutation. Existing
                    // data remains readable; uncertain write failures still poison.
                    assert!(volume.revision().is_ok());
                    committed = count;
                    break;
                }
                other => panic!("unexpected capacity result {other:?}"),
            }
        }
        // Drop the unpublished allocation attempt; retain the prior checkpoint.
    }
    let (mut journal, report) =
        Journal::open(&mut *disk, home, log, *b"DoorP07DiskGUID0", work).unwrap();
    assert!(!report.replayed);
    journal.begin().unwrap();
    let mut superblock = [0; 4096];
    journal.read_sectors(0, &mut superblock).unwrap();
    assert_eq!(
        u32::from_le_bytes(superblock[1036..1040].try_into().unwrap()),
        0
    );
    let mut volume =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), state, catalog).unwrap();
    for index in 0..committed {
        let mut name = *b"/users/1001/full00";
        name[16] = b'0' + index / 10;
        name[17] = b'0' + index % 10;
        let (file, _) = volume
            .open(
                subject(1001),
                Path::Native(core::str::from_utf8(&name).unwrap()),
                Context::root(),
                options(Access::READ, Disposition::OpenExisting),
            )
            .unwrap();
        let mut bytes = [0; 8192];
        assert_eq!(volume.read(&file, 0, &mut bytes).unwrap(), bytes.len());
        assert!(bytes.iter().all(|&b| b == 0x40 + index));
        volume.close(file).unwrap();
    }
    let mut absent = *b"/users/1001/full00";
    absent[16] = b'0' + committed / 10;
    absent[17] = b'0' + committed % 10;
    assert!(matches!(
        volume.open(
            subject(1001),
            Path::Native(core::str::from_utf8(&absent).unwrap()),
            Context::root(),
            options(Access::READ, Disposition::OpenExisting)
        ),
        Err(Error::NotFound)
    ));
    let fs = volume.into_inner().unwrap();
    let mut journal = fs.into_inner().unwrap();
    journal.abort().unwrap();
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_P07 CAPACITY PASS files={committed} free_blocks=0 rollback=1 preserved=1"
    )
    .ok();
}
