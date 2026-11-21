//! Staged whole-file replacement: atomic publication at every journal
//! boundary, interrupted staging, abort and bounded refusals.
use super::*;
use crate::storage::{Checkpoint, Journal, WorkingSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

const OLD: usize = 4100;
/// Past the 12 direct blocks: the candidate needs the single-indirect block.
const NEW: usize = 13 * 4096 + 100;
const DOC: &str = "/users/1001/doc.txt";

fn open(v: &mut PersistentVolume<'_, impl Vfs>, access: Access) -> FileHandle {
    let o = OpenOptions {
        access,
        share: Access::SHARE_ALL,
        disposition: Disposition::OpenExisting,
    };
    v.open(user(1001), Path::Native(DOC), Context::root(), o)
        .unwrap()
        .0
}
fn contents(v: &mut PersistentVolume<'_, impl Vfs>) -> (FileInfo, Vec<u8>) {
    let file = open(v, Access::READ);
    let info = v.info(&file).unwrap();
    let mut out = std::vec![0; info.bytes as usize];
    assert_eq!(v.read(&file, 0, &mut out).unwrap(), out.len());
    v.close(file).unwrap();
    (info, out)
}
fn no_staging(v: &PersistentVolume<'_, impl Vfs>) -> bool {
    v.state
        .nodes
        .iter()
        .all(|n| n.id == 0 || (!n.pending && n.parent != 0))
}
/// A journaled volume with DOC holding OLD bytes of 'a', committed.
fn baseline() -> crate::storage::tests::Disk {
    let (home, log) = partitions();
    let mut disk = journal_disk();
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
    journal.begin().unwrap();
    let mut v = PersistentVolume::initialize(
        Ext2::mount(journal).unwrap(),
        &mut state,
        &mut catalog,
        system(),
    )
    .unwrap();
    v.mkdir(
        system(),
        Path::Native("/users/1001"),
        Context::root(),
        Some(Acl::owner_full(1001)),
    )
    .unwrap();
    let (file, _) = v
        .open(
            user(1001),
            Path::Native(DOC),
            Context::root(),
            options(Disposition::CreateNew),
        )
        .unwrap();
    v.write(&file, 0, &[b'a'; OLD]).unwrap();
    v.close(file).unwrap();
    v.commit().unwrap();
    disk
}
fn stage_all(
    v: &mut PersistentVolume<'_, Ext2<Journal<'_, &mut crate::storage::tests::Disk>>>,
    file: &FileHandle,
) -> Staged {
    let info = v.info(file).unwrap();
    let mut staged = v
        .stage_replace(file, info.object, info.version, NEW as u64)
        .unwrap();
    for (i, chunk) in [b'b'; NEW].chunks(4096).enumerate() {
        v.stage_write(&mut staged, (i * 4096) as u64, chunk)
            .unwrap();
    }
    staged
}

#[test]
fn staged_replacement_is_atomic_at_every_journal_boundary() {
    let (home, log) = partitions();
    let base = baseline();
    let mut work = Box::new(WorkingSet::new());
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    // Uninterrupted: same object, version + 1, new length and times.
    {
        let mut disk = base.clone();
        let mut state = State::boxed();
        let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
        journal.begin().unwrap();
        let mut v =
            PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog)
                .unwrap();
        let (old, _) = contents(&mut v);
        let file = open(&mut v, Access::ALL);
        let staged = stage_all(&mut v, &file);
        // The candidate is invisible until the commit.
        assert_eq!(contents(&mut v).1, [b'a'; OLD]);
        v.set_clock(133_000_000_000_000_000);
        let info = v.commit_staged(&file, &staged).unwrap();
        assert_eq!(
            (info.object, info.version, info.bytes),
            (old.object, old.version + 1, NEW as u64)
        );
        assert_eq!(info.times.modified, 133_000_000_000_000_000);
        assert!(!v.staged_live(&staged) && no_staging(&v));
        v.close(file).unwrap();
        let (now, bytes) = contents(&mut v);
        assert_eq!(
            (now.object, now.version, bytes),
            (old.object, old.version + 1, std::vec![b'b'; NEW])
        );
    }
    for checkpoint in [
        Checkpoint::PrepareFlushed,
        Checkpoint::CommitFlushed,
        Checkpoint::FirstHomeFlushed,
        Checkpoint::HomeFlushed,
        Checkpoint::CheckpointFlushed,
    ] {
        let mut disk = base.clone();
        let mut state = State::boxed();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
            journal.begin().unwrap();
            let mut v =
                PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog)
                    .unwrap();
            let file = open(&mut v, Access::ALL);
            let staged = stage_all(&mut v, &file);
            v.commit_staged_with(&file, &staged, |point| {
                if point == checkpoint {
                    panic!("power cut");
                }
            })
            .unwrap();
        }));
        assert!(result.is_err());
        // Recovery (twice: replay is idempotent) yields old or new whole.
        for _ in 0..2 {
            let mut state = State::boxed();
            let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
            journal.begin().unwrap();
            let mut v =
                PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog)
                    .unwrap();
            let (info, bytes) = contents(&mut v);
            if checkpoint == Checkpoint::PrepareFlushed {
                assert_eq!(
                    (info.version, bytes),
                    (1 + 1, std::vec![b'a'; OLD]),
                    "{checkpoint:?}"
                );
            } else {
                assert_eq!(
                    (info.version, bytes),
                    (2 + 1, std::vec![b'b'; NEW]),
                    "{checkpoint:?}"
                );
            }
            v.commit().unwrap();
            assert!(no_staging(&v), "{checkpoint:?}");
        }
    }
}

#[test]
fn interrupted_staging_and_abort_keep_the_original_and_reclaim_the_candidate() {
    let (home, log) = partitions();
    let mut disk = baseline();
    let mut work = Box::new(WorkingSet::new());
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let free_before;
    {
        let mut state = State::boxed();
        let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
        journal.begin().unwrap();
        let mut v =
            PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog)
                .unwrap();
        free_before = v.usage().unwrap().1;
        let file = open(&mut v, Access::ALL);
        let info = v.info(&file).unwrap();
        let mut staged = v
            .stage_replace(&file, info.object, info.version, NEW as u64)
            .unwrap();
        for i in 0..6 {
            v.stage_write(&mut staged, i * 4096, &[b'b'; 4096]).unwrap();
        }
        // Part of the candidate is durable, then power is cut mid-upload.
        v.commit().unwrap();
        assert!(v.usage().unwrap().1 < free_before);
    }
    {
        let mut state = State::boxed();
        let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
        journal.begin().unwrap();
        let mut v =
            PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog)
                .unwrap();
        // The parentless pending node was reclaimed by the validated mount.
        assert!(no_staging(&v));
        assert_eq!(contents(&mut v).1, [b'a'; OLD]);
        v.commit().unwrap();
        assert_eq!(v.usage().unwrap().1, free_before);
        // Explicit abort; refused commits keep the candidate for abort.
        let file = open(&mut v, Access::ALL);
        let info = v.info(&file).unwrap();
        let mut staged = v
            .stage_replace(&file, info.object, info.version, 5000)
            .unwrap();
        v.stage_write(&mut staged, 0, &[b'c'; 4096]).unwrap();
        assert_eq!(
            v.commit_staged(&file, &staged),
            Err(Error::InvalidParameter)
        );
        v.stage_write(&mut staged, 4096, &[b'c'; 904]).unwrap();
        v.write(&file, 0, b"A").unwrap(); // the target changed meanwhile
        assert_eq!(v.commit_staged(&file, &staged), Err(Error::Changed));
        assert!(v.staged_live(&staged));
        v.abort_staged(staged).unwrap();
        v.commit().unwrap();
        v.close(file).unwrap();
        assert!(no_staging(&v));
        assert_eq!(v.usage().unwrap().1, free_before);
        let (_, bytes) = contents(&mut v);
        assert_eq!((bytes.len(), bytes[0], bytes[1]), (OLD, b'A', b'a'));
    }
}

#[test]
fn over_limit_and_capacity_refusals_leave_the_owner_usable() {
    let (home, log) = partitions();
    let mut disk = baseline();
    let mut work = Box::new(WorkingSet::new());
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let mut state = State::boxed();
    let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
    journal.begin().unwrap();
    let mut v =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    let file = open(&mut v, Access::ALL);
    let info = v.info(&file).unwrap();
    let revision = v.revision().unwrap();
    let limit = LARGE_REPLACE_BYTES as u64;
    assert_eq!(
        v.stage_replace(&file, info.object, info.version, limit + 1)
            .unwrap_err(),
        Error::InvalidParameter
    );
    // Within the limit but beyond this volume's free space.
    assert_eq!(
        v.stage_replace(&file, info.object, info.version, limit)
            .unwrap_err(),
        Error::NoSpace
    );
    assert_eq!(
        v.stage_replace(&file, info.object, info.version + 1, 1)
            .unwrap_err(),
        Error::Changed
    );
    let reader = open(&mut v, Access::READ);
    assert_eq!(
        v.stage_replace(&reader, info.object, info.version, 1)
            .unwrap_err(),
        Error::AccessDenied
    );
    assert_eq!(v.revision().unwrap(), revision, "refusals stage nothing");
    // Writes: sequential, 1..=4096 bytes, within the declared length.
    let mut staged = v
        .stage_replace(&file, info.object, info.version, 4097)
        .unwrap();
    for (offset, len) in [(1, 1), (0, 0), (0, 4097)] {
        assert_eq!(
            v.stage_write(&mut staged, offset, &std::vec![1; len]),
            Err(Error::InvalidParameter)
        );
    }
    v.stage_write(&mut staged, 0, &[1; 4096]).unwrap();
    assert_eq!(
        v.stage_write(&mut staged, 4096, &[1; 2]),
        Err(Error::InvalidParameter)
    );
    v.stage_write(&mut staged, 4096, &[1; 1]).unwrap();
    let info = v.commit_staged(&file, &staged).unwrap();
    assert_eq!(info.bytes, 4097);
    // A consumed candidate cannot be reused.
    assert_eq!(
        v.stage_write(&mut staged, 4097, &[1]),
        Err(Error::InvalidHandle)
    );
    assert_eq!(v.commit_staged(&file, &staged), Err(Error::InvalidHandle));
    assert_eq!(v.abort_staged(staged), Err(Error::InvalidHandle));
    v.close(reader).unwrap();
    v.close(file).unwrap();
    assert!(no_staging(&v));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Mount,
    Begin,
    Chunk(usize),
    /// The pre-swap commit of every candidate page, up to the swap's
    /// prepare records being flushed.
    PreSwap,
    /// The swap transaction's commit record and home writes.
    Swap,
    /// Close and commit after the acknowledged swap.
    PostSwap,
}
/// Device write counts at the end of each phase of one staged replacement.
#[derive(Default)]
struct Marks {
    phases: Vec<(Phase, usize)>,
    /// Writes when the swap's prepare records / commit record were flushed.
    prepared: usize,
    durable: usize,
}
/// Replace DOC by NEW bytes of 'b' through the staged path, stopping at the
/// first error (the power cut) and naming its phase. `interleave` commits
/// after begin and every chunk, as other clients' requests (FLUSH, budget
/// checkpoints) do in the service, so those steps reach the device too.
fn staged_run(
    disk: &mut crate::storage::tests::Disk,
    work: &mut WorkingSet,
    catalog: &mut [u8; CATALOG_BYTES],
    writes: &std::cell::Cell<usize>,
    marks: &mut Marks,
    interleave: bool,
) -> core::result::Result<(), Phase> {
    let (home, log) = partitions();
    let mut state = State::boxed();
    let (mut journal, _) =
        Journal::open(disk, home, log, [7; 16], work).map_err(|_| Phase::Mount)?;
    journal.begin().map_err(|_| Phase::Mount)?;
    let fs = Ext2::mount(journal).map_err(|_| Phase::Mount)?;
    let mut v = PersistentVolume::mount(fs, &mut state, catalog).map_err(|_| Phase::Mount)?;
    let o = OpenOptions {
        access: Access::ALL,
        share: Access::SHARE_ALL,
        disposition: Disposition::OpenExisting,
    };
    let (file, _) = v
        .open(user(1001), Path::Native(DOC), Context::root(), o)
        .map_err(|_| Phase::Mount)?;
    let info = v.info(&file).map_err(|_| Phase::Mount)?;
    marks.phases.push((Phase::Mount, writes.get()));
    let mut staged = v
        .stage_replace(&file, info.object, info.version, NEW as u64)
        .map_err(|_| Phase::Begin)?;
    if interleave {
        v.commit().map_err(|_| Phase::Begin)?;
    }
    marks.phases.push((Phase::Begin, writes.get()));
    for (i, chunk) in [b'b'; NEW].chunks(4096).enumerate() {
        v.stage_write(&mut staged, (i * 4096) as u64, chunk)
            .map_err(|_| Phase::Chunk(i))?;
        if interleave {
            v.commit().map_err(|_| Phase::Chunk(i))?;
        }
        marks.phases.push((Phase::Chunk(i), writes.get()));
    }
    let (mut prepared, mut durable) = (None, None);
    let result = v.commit_staged_with(&file, &staged, |point| match point {
        Checkpoint::PrepareFlushed => prepared = Some(writes.get()),
        Checkpoint::CommitFlushed => durable = Some(writes.get()),
        _ => (),
    });
    marks.prepared = prepared.unwrap_or(0);
    marks.durable = durable.unwrap_or(0);
    result.map_err(|_| match prepared {
        None => Phase::PreSwap,
        Some(_) => Phase::Swap,
    })?;
    marks.phases.push((Phase::PreSwap, marks.prepared));
    marks.phases.push((Phase::Swap, writes.get()));
    v.close(file).map_err(|_| Phase::PostSwap)?;
    v.commit().map_err(|_| Phase::PostSwap)?;
    marks.phases.push((Phase::PostSwap, writes.get()));
    Ok(())
}
/// Remount (journal replay plus validated catalog mount) and commit the
/// recovery: DOC's identity and bytes, free bytes, and whether no staging
/// node survived.
fn recover(
    disk: &mut crate::storage::tests::Disk,
    work: &mut WorkingSet,
    catalog: &mut [u8; CATALOG_BYTES],
) -> (FileInfo, Vec<u8>, u64, bool) {
    let (home, log) = partitions();
    let mut state = State::boxed();
    let (mut journal, _) = Journal::open(disk, home, log, [7; 16], work).expect("journal opens");
    journal.begin().unwrap();
    let fs = Ext2::mount(journal).expect("ext2 mounts");
    let mut v = PersistentVolume::mount(fs, &mut state, catalog).expect("catalog mounts");
    let (info, bytes) = contents(&mut v);
    v.commit().unwrap();
    (info, bytes, v.usage().unwrap().1, no_staging(&v))
}

/// Power is cut at every device write of a staged replacement (mount,
/// begin, each chunk with its interim commits, the pre-swap commit, the
/// swap and the post-swap close/commit), both cleanly and with a torn
/// write. Every cut disk remounts (twice: replay is idempotent) to the
/// complete old or complete new file, never a mix, never new-then-old, with
/// no staging node and no leaked block. A cut before the swap's prepare is
/// flushed is old; one after its commit record is flushed, new.
#[test]
fn power_cut_at_every_staged_replace_write_recovers_old_or_new() {
    power_cut_sweep(false);
}
/// As above with a commit after begin and after every chunk, so each of
/// those steps is cut mid-journal as well.
#[test]
fn power_cut_at_every_interleaved_staging_commit_recovers_old_or_new() {
    let cuts = power_cut_sweep(true);
    for phase in core::iter::once(Phase::Begin).chain((0..NEW.div_ceil(4096)).map(Phase::Chunk)) {
        assert!(
            cuts.iter().any(|(p, n)| *p == phase && *n > 0),
            "no cut inside {phase:?}"
        );
    }
}
fn power_cut_sweep(interleave: bool) -> Vec<(Phase, usize)> {
    let base = baseline();
    let mut work = Box::new(WorkingSet::new());
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let (old, old_bytes, old_free, _) = recover(&mut base.clone(), &mut work, &mut catalog);
    let mut reference = base.clone();
    let writes = reference.power_cut_after(None, false);
    let mut marks = Marks::default();
    staged_run(
        &mut reference,
        &mut work,
        &mut catalog,
        &writes,
        &mut marks,
        interleave,
    )
    .unwrap();
    let total = writes.get();
    let (new, new_bytes, new_free, clean) = recover(&mut reference, &mut work, &mut catalog);
    assert!(clean);
    assert_eq!(old_bytes, std::vec![b'a'; OLD]);
    assert_eq!(new_bytes, std::vec![b'b'; NEW]);
    assert_eq!((new.object, new.version), (old.object, old.version + 1));
    let chunks = NEW.div_ceil(4096);
    assert_eq!(marks.phases.len(), 2 + chunks + 3, "every phase completed");
    assert!(0 < marks.prepared && marks.prepared < marks.durable && marks.durable <= total);
    let phase_of = |n: usize| {
        marks
            .phases
            .iter()
            .find(|(_, end)| n <= *end)
            .map_or(Phase::PostSwap, |(phase, _)| *phase)
    };
    let mut cuts_by_phase: Vec<(Phase, usize)> = Vec::new();
    for torn in [false, true] {
        let mut seen_new = false;
        for n in 0..=total {
            let mut disk = base.clone();
            let counter = disk.power_cut_after(Some(n), torn);
            let mut m = Marks::default();
            let stopped = staged_run(
                &mut disk,
                &mut work,
                &mut catalog,
                &counter,
                &mut m,
                interleave,
            );
            assert!(stopped.is_err(), "cut {n} (torn {torn}) was acknowledged");
            let phase = phase_of(n.max(1));
            match cuts_by_phase.iter_mut().find(|(p, _)| *p == phase) {
                Some((_, count)) => *count += 1,
                None => cuts_by_phase.push((phase, 1)),
            }
            disk.power_cut_after(None, false);
            let first = recover(&mut disk, &mut work, &mut catalog);
            let again = recover(&mut disk, &mut work, &mut catalog);
            assert_eq!(
                (&first.0, &first.1),
                (&again.0, &again.1),
                "replay is idempotent"
            );
            let (info, bytes, free, clean) = first;
            assert!(clean, "cut {n} ({phase:?}) left a staging node");
            let is_new = info.version == new.version;
            if is_new {
                assert_eq!(bytes, new_bytes, "cut {n} ({phase:?})");
                assert_eq!(free, new_free, "cut {n} ({phase:?}) leaked blocks");
            } else {
                assert_eq!(info.version, old.version, "cut {n} ({phase:?})");
                assert_eq!(bytes, std::vec![b'a'; OLD], "cut {n} ({phase:?})");
                assert_eq!(free, old_free, "cut {n} ({phase:?}) leaked blocks");
            }
            assert_eq!(info.object, old.object);
            if n <= marks.prepared {
                assert!(!is_new, "cut {n} ({phase:?}) published before the swap");
            }
            if n > marks.durable {
                assert!(is_new, "cut {n} ({phase:?}) lost an acknowledged swap");
            }
            assert!(!seen_new || is_new, "cut {n}: new then old");
            seen_new |= is_new;
        }
        assert!(seen_new);
    }
    for phase in [
        Phase::Mount,
        Phase::Begin,
        Phase::PreSwap,
        Phase::Swap,
        Phase::PostSwap,
    ]
    .into_iter()
    .chain((0..chunks).map(Phase::Chunk))
    {
        assert!(
            marks.phases.iter().any(|(p, _)| *p == phase),
            "{phase:?} boundary is a cut point"
        );
    }
    std::println!(
        "staged replace (interleave {interleave}): {total} writes, cuts by phase {cuts_by_phase:?}"
    );
    cuts_by_phase
}

/// Concurrent staged candidates reserve their blocks: a second candidate
/// that fits only if the first never finishes is refused at begin, and the
/// reservation drains as blocks are written and on commit/abort.
#[test]
fn staged_candidates_reserve_free_space_until_written_or_aborted() {
    let (home, log) = partitions();
    let mut disk = baseline();
    let mut work = Box::new(WorkingSet::new());
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let mut state = State::boxed();
    let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
    journal.begin().unwrap();
    let mut v =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    let file = open(&mut v, Access::ALL);
    let info = v.info(&file).unwrap();
    let free_blocks = v.usage().unwrap().1 / 4096;
    // The first candidate takes all but two free blocks; a second of three
    // blocks would have passed the old unreserved check.
    let bytes = (free_blocks - 3) * 4096;
    let mut first = v
        .stage_replace(&file, info.object, info.version, bytes)
        .unwrap();
    let reserved = v.staged_reserved_blocks();
    assert_eq!(reserved, free_blocks - 2);
    assert_eq!(
        v.stage_replace(&file, info.object, info.version, 3 * 4096)
            .unwrap_err(),
        Error::NoSpace,
        "the first candidate's blocks are promised"
    );
    v.stage_write(&mut first, 0, &[b'r'; 4096]).unwrap();
    assert_eq!(v.staged_reserved_blocks(), reserved - 1);
    v.abort_staged(first).unwrap();
    assert_eq!(v.staged_reserved_blocks(), 0);
    let mut second = v
        .stage_replace(&file, info.object, info.version, 4097)
        .unwrap();
    v.stage_write(&mut second, 0, &[1; 4096]).unwrap();
    v.stage_write(&mut second, 4096, &[1]).unwrap();
    assert_eq!(
        v.staged_reserved_blocks(),
        0,
        "a written candidate holds nothing"
    );
    v.commit_staged(&file, &second).unwrap();
    assert_eq!(v.staged_reserved_blocks(), 0);
    v.close(file).unwrap();
}
