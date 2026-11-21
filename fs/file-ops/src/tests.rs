//! Engine behaviour on the journaled host model: every mode, conflicts,
//! cancellation, pause/resume, disk full, and a crash at every mutation and
//! commit followed by recovery and resume.
use crate::memory::Mem;
use crate::{Bin, Decision, Engine, Error, Mode, Policy, RecycleOutcome, State, Volume};
use std::boxed::Box;
use std::string::String;
use std::vec;
use std::vec::Vec;

const U: u32 = 1001;
fn fixture() -> Mem {
    let mut m = Mem::new();
    m.mkdir("/users/1001/src").unwrap();
    m.mkdir("/users/1001/dst").unwrap();
    m.write_file("/users/1001/src/a.txt", b"alpha");
    m.mkdir("/users/1001/src/tree").unwrap();
    m.write_file("/users/1001/src/tree/big.bin", &pattern(150_000));
    m.mkdir("/users/1001/src/tree/sub").unwrap();
    m.write_file("/users/1001/src/tree/sub/c.txt", b"gamma");
    m.commit().unwrap();
    m
}
fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 % 251) as u8).collect()
}
fn start(e: &mut Engine, m: &mut Mem, op: u64, mode: Mode, policy: Policy, dst: &str) {
    e.begin(m, op, U, mode, policy, dst).unwrap();
    for s in ["/users/1001/src/a.txt", "/users/1001/src/tree"] {
        e.add(m, op, U, s).unwrap();
    }
}
fn run(e: &mut Engine, m: &mut Mem, op: u64) -> crate::Progress {
    for _ in 0..1000 {
        let p = e.step(m, op, U).unwrap();
        if p.state != State::Running && p.state != State::Scanning {
            return p;
        }
    }
    panic!("operation did not finish");
}
fn subtree(m: &Mem, root: &str) -> Vec<(String, Option<Vec<u8>>)> {
    m.tree()
        .into_iter()
        .filter_map(|(p, d)| p.strip_prefix(root).map(|r| (String::from(r), d)))
        .filter(|(p, _)| !p.is_empty())
        .collect()
}

#[test]
fn copy_tree_reports_progress_and_reproduces_every_byte() {
    let mut m = fixture();
    let mut e = Box::new(Engine::new());
    start(
        &mut e,
        &mut m,
        7,
        Mode::Copy,
        Policy::Ask,
        "/users/1001/dst",
    );
    let scan = e.step(&mut m, 7, U).unwrap();
    assert_eq!(scan.state, State::Running);
    assert_eq!((scan.items_total, scan.bytes_total), (5, 150_010));
    let mut steps = 0;
    let p = loop {
        let p = e.step(&mut m, 7, U).unwrap();
        steps += 1;
        assert!(p.bytes_done <= p.bytes_total);
        if p.state != State::Running {
            break p;
        }
    };
    assert!(steps >= 3, "large file spans bounded steps");
    assert_eq!(p.state, State::Done);
    assert_eq!(
        (p.items_done, p.bytes_done, p.skipped, p.failures),
        (5, 150_010, 0, 0)
    );
    assert_eq!(
        subtree(&m, "/users/1001/src"),
        subtree(&m, "/users/1001/dst")
    );
    // No temporary file remains anywhere.
    assert!(m.tree().iter().all(|(p, _)| !p.contains("~$fop")));
    // The record survives until dismissed; its ID is private to the owner.
    assert_eq!(e.query(&mut m, 7, 1002), Err(Error::NotFound));
    e.dismiss(&mut m, 7, U).unwrap();
    assert_eq!(e.query(&mut m, 7, U), Err(Error::NotFound));
}

#[test]
fn move_renames_atomically_and_merges_into_existing_folders() {
    let mut m = fixture();
    m.mkdir("/users/1001/dst/tree").unwrap();
    m.write_file("/users/1001/dst/tree/keep.txt", b"keep");
    m.commit().unwrap();
    let before = subtree(&m, "/users/1001/src");
    let mut e = Box::new(Engine::new());
    start(
        &mut e,
        &mut m,
        8,
        Mode::Move,
        Policy::Ask,
        "/users/1001/dst",
    );
    let p = run(&mut e, &mut m, 8);
    assert_eq!(p.state, State::Done);
    assert_eq!(m.names("/users/1001/src"), Vec::<String>::new());
    let after = subtree(&m, "/users/1001/dst");
    for item in &before {
        assert!(after.contains(item), "{item:?} moved");
    }
    assert!(after.iter().any(|(p, _)| p == "/tree/keep.txt"));
    assert_eq!((p.items_done, p.bytes_done), (p.items_total, p.bytes_total));
}

#[test]
fn conflicts_ask_then_apply_to_all_and_keep_both_naming() {
    let mut m = fixture();
    m.write_file("/users/1001/dst/a.txt", b"old");
    m.mkdir("/users/1001/dst/tree").unwrap();
    m.write_file("/users/1001/dst/tree/big.bin", b"old");
    m.commit().unwrap();
    let mut e = Box::new(Engine::new());
    start(
        &mut e,
        &mut m,
        9,
        Mode::Copy,
        Policy::Ask,
        "/users/1001/dst",
    );
    let p = run(&mut e, &mut m, 9);
    assert_eq!(p.state, State::Conflict);
    assert_eq!(p.current.as_str(), "a.txt");
    let (src, dst) = p.conflict.unwrap();
    assert_eq!((src.bytes, dst.bytes), (5, 3));
    // Steps do nothing while the decision is pending.
    assert_eq!(e.step(&mut m, 9, U).unwrap().state, State::Conflict);
    e.resolve(&mut m, 9, U, Decision::KeepBoth, false).unwrap();
    let p = run(&mut e, &mut m, 9);
    assert_eq!(p.state, State::Conflict);
    assert_eq!(p.current.as_str(), "big.bin");
    e.resolve(&mut m, 9, U, Decision::Replace, true).unwrap();
    let p = run(&mut e, &mut m, 9);
    assert_eq!(p.state, State::Done);
    assert_eq!(m.read_file("/users/1001/dst/a.txt"), b"old");
    assert_eq!(m.read_file("/users/1001/dst/a (2).txt"), b"alpha");
    assert_eq!(
        m.read_file("/users/1001/dst/tree/big.bin"),
        pattern(150_000)
    );
    // Skip policy leaves destinations alone and counts the skipped items.
    let mut e = Box::new(Engine::new());
    start(
        &mut e,
        &mut m,
        10,
        Mode::Copy,
        Policy::Skip,
        "/users/1001/dst",
    );
    let p = run(&mut e, &mut m, 10);
    assert_eq!(p.state, State::Done);
    assert_eq!(p.skipped, 3, "a.txt, big.bin, c.txt");
    assert_eq!(m.read_file("/users/1001/dst/a.txt"), b"old");
}

#[test]
fn same_folder_copy_names_copies_and_self_nesting_is_refused() {
    let mut m = fixture();
    let mut e = Box::new(Engine::new());
    e.begin(&mut m, 11, U, Mode::Copy, Policy::Ask, "/users/1001/src")
        .unwrap();
    e.add(&mut m, 11, U, "/users/1001/src/a.txt").unwrap();
    assert_eq!(run(&mut e, &mut m, 11).state, State::Done);
    e.begin(&mut m, 12, U, Mode::Copy, Policy::Ask, "/users/1001/src")
        .unwrap();
    e.add(&mut m, 12, U, "/users/1001/src/a.txt").unwrap();
    assert_eq!(run(&mut e, &mut m, 12).state, State::Done);
    assert_eq!(m.read_file("/users/1001/src/a - Copy.txt"), b"alpha");
    assert_eq!(m.read_file("/users/1001/src/a - Copy (2).txt"), b"alpha");
    e.begin(
        &mut m,
        13,
        U,
        Mode::Copy,
        Policy::Ask,
        "/users/1001/src/tree/sub",
    )
    .unwrap();
    assert_eq!(
        e.add(&mut m, 13, U, "/users/1001/src/tree"),
        Err(Error::Invalid)
    );
    // Sources must share one folder; duplicates are refused.
    e.add(&mut m, 13, U, "/users/1001/src/a.txt").unwrap();
    assert_eq!(
        e.add(&mut m, 13, U, "/users/1001/src/a.txt"),
        Err(Error::Invalid)
    );
    assert_eq!(e.add(&mut m, 13, U, "/users/1001/dst"), Err(Error::Invalid));
}

#[test]
fn cancel_discards_the_partial_item_and_keeps_finished_ones() {
    let mut m = fixture();
    let mut e = Box::new(Engine::new());
    start(
        &mut e,
        &mut m,
        14,
        Mode::Copy,
        Policy::Ask,
        "/users/1001/dst",
    );
    e.step(&mut m, 14, U).unwrap(); // scan
    loop {
        let p = e.step(&mut m, 14, U).unwrap();
        if p.bytes_done > 5 + 4096 {
            break;
        }
    }
    assert!(m.tree().iter().any(|(p, _)| p.contains("~$fop")));
    let p = e.cancel(&mut m, 14, U).unwrap();
    assert_eq!(p.state, State::Cancelled);
    assert!(m.tree().iter().all(|(p, _)| !p.contains("~$fop")));
    assert_eq!(m.read_file("/users/1001/dst/a.txt"), b"alpha");
    assert!(!m.exists("/users/1001/dst/tree/big.bin"));
    // Cancelled is final; the durable state agrees after a crash.
    m.crash();
    assert_eq!(e.query(&mut m, 14, U).unwrap().state, State::Cancelled);
    assert!(m.tree().iter().all(|(p, _)| !p.contains("~$fop")));
}

#[test]
fn pause_is_durable_and_resume_continues_the_partial_file() {
    let mut m = fixture();
    let mut e = Box::new(Engine::new());
    start(
        &mut e,
        &mut m,
        15,
        Mode::Copy,
        Policy::Ask,
        "/users/1001/dst",
    );
    e.step(&mut m, 15, U).unwrap();
    e.step(&mut m, 15, U).unwrap();
    e.step(&mut m, 15, U).unwrap();
    e.pause(&mut m, 15, U).unwrap();
    let paused = e.step(&mut m, 15, U).unwrap();
    assert_eq!(paused.state, State::Paused);
    m.crash();
    let mut e = Box::new(Engine::new());
    assert_eq!(e.query(&mut m, 15, U).unwrap(), paused);
    e.resume(&mut m, 15, U).unwrap();
    assert_eq!(run(&mut e, &mut m, 15).state, State::Done);
    assert_eq!(
        subtree(&m, "/users/1001/src"),
        subtree(&m, "/users/1001/dst")
    );
}

#[test]
fn disk_full_fails_truthfully_without_leaving_a_partial_file() {
    let mut m = fixture();
    m.capacity = m
        .tree()
        .iter()
        .filter_map(|(_, d)| d.as_ref())
        .map(|d| d.len() as u64)
        .sum::<u64>()
        + 60_000;
    let mut e = Box::new(Engine::new());
    start(
        &mut e,
        &mut m,
        16,
        Mode::Copy,
        Policy::Ask,
        "/users/1001/dst",
    );
    let p = run(&mut e, &mut m, 16);
    assert_eq!(p.state, State::Failed);
    assert_eq!(p.last_error, Some(Error::NoSpace));
    assert!(m.tree().iter().all(|(p, _)| !p.contains("~$fop")));
    assert_eq!(m.read_file("/users/1001/dst/a.txt"), b"alpha");
    assert!(!m.exists("/users/1001/dst/tree/big.bin"));
    m.crash();
    assert!(m.tree().iter().all(|(p, _)| !p.contains("~$fop")));
}

#[test]
fn permanent_delete_removes_trees_post_order() {
    let mut m = fixture();
    let mut e = Box::new(Engine::new());
    e.begin(&mut m, 17, U, Mode::Delete, Policy::Ask, "")
        .unwrap();
    e.add(&mut m, 17, U, "/users/1001/src/tree").unwrap();
    e.add(&mut m, 17, U, "/users/1001/src/a.txt").unwrap();
    let p = run(&mut e, &mut m, 17);
    assert_eq!(p.state, State::Done);
    assert_eq!((p.items_done, p.items_total), (5, 5));
    assert_eq!(m.names("/users/1001/src"), Vec::<String>::new());
}

/// Crash at every mutation and commit of a full copy, move and delete;
/// after recovery the operation resumes and the end state is identical to
/// an uninterrupted run, with no temporary file left.
#[test]
fn crash_at_every_point_recovers_and_resumes_to_the_same_result() {
    for mode in [Mode::Copy, Mode::Move, Mode::Delete] {
        let dst = if mode == Mode::Delete {
            ""
        } else {
            "/users/1001/dst"
        };
        let mut clean = fixture();
        let mut e = Box::new(Engine::new());
        start(&mut e, &mut clean, 20, mode, Policy::Ask, dst);
        let base = clean.mutations;
        assert_eq!(run(&mut e, &mut clean, 20).state, State::Done);
        let expected = clean.tree();
        let total = clean.mutations - base;
        for cut in 1..=total {
            let mut m = fixture();
            let mut e = Box::new(Engine::new());
            start(&mut e, &mut m, 20, mode, Policy::Ask, dst);
            m.fail_at = Some(m.mutations + cut);
            let mut crashed = false;
            for _ in 0..1000 {
                match e.step(&mut m, 20, U) {
                    Ok(p) if p.state == State::Done => break,
                    Ok(_) => (),
                    Err(err) => {
                        assert!(err.fatal(), "{mode:?} cut {cut}: {err:?}");
                        crashed = true;
                        break;
                    }
                }
            }
            assert!(crashed, "{mode:?} cut {cut} reached");
            m.crash();
            let mut e = Box::new(Engine::new());
            let recovered = e.recover(&mut m).unwrap();
            if e.query(&mut m, 20, U) == Err(Error::NotFound) {
                // Cut before the scan committed: the unstarted op was dropped.
                start(&mut e, &mut m, 20, mode, Policy::Ask, dst);
            }
            let state = e.query(&mut m, 20, U).unwrap().state;
            assert!(
                matches!(state, State::Paused | State::Done | State::Scanning),
                "{mode:?} cut {cut}: {state:?} ({recovered})"
            );
            if state == State::Paused {
                e.resume(&mut m, 20, U).unwrap();
            }
            assert_eq!(run(&mut e, &mut m, 20).state, State::Done);
            assert_eq!(m.tree(), expected, "{mode:?} cut {cut}");
        }
    }
}

#[test]
fn corrupt_records_are_never_replayed() {
    let mut m = fixture();
    let mut e = Box::new(Engine::new());
    start(
        &mut e,
        &mut m,
        21,
        Mode::Copy,
        Policy::Ask,
        "/users/1001/dst",
    );
    let mut bytes = m.live.records[&21].clone();
    for at in [0usize, 12, 28, 30, 67, 240, bytes.len() - 1] {
        let mut bad = bytes.clone();
        bad[at] ^= 0x41;
        m.live.records.insert(21, bad);
        assert_eq!(e.step(&mut m, 21, U), Err(Error::Corrupt), "byte {at}");
    }
    bytes.truncate(bytes.len() - 3);
    m.live.records.insert(21, bytes);
    assert_eq!(e.query(&mut m, 21, U), Err(Error::Corrupt));
    assert_eq!(e.recover(&mut m).unwrap(), 0);
    assert!(!m.exists("/users/1001/dst/a.txt"));
}

#[test]
fn recycle_restore_purge_and_limits() {
    let mut m = fixture();
    let bin = Bin {
        uid: U,
        limit: 200_000,
    };
    let RecycleOutcome::Recycled(a) = bin.recycle(&mut m, "/users/1001/src/a.txt").unwrap() else {
        panic!()
    };
    let RecycleOutcome::Recycled(t) = bin.recycle(&mut m, "/users/1001/src/tree").unwrap() else {
        panic!()
    };
    assert_eq!(m.names("/users/1001/src"), Vec::<String>::new());
    assert!(
        m.live
            .nodes
            .values()
            .any(|n| n.name == "$Recycle.Bin" && n.hidden)
    );
    let first = bin.next(&mut m, 0).unwrap().unwrap();
    assert_eq!(
        (
            first.id,
            first.original.as_str(),
            first.bytes,
            first.directory
        ),
        (a, "/users/1001/src/a.txt", 5, false)
    );
    assert_eq!(first.deleted, m.clock);
    let second = bin.next(&mut m, a).unwrap().unwrap();
    assert_eq!((second.id, second.items, second.directory), (t, 4, true));
    assert!(bin.next(&mut m, t).unwrap().is_none());
    assert_eq!(bin.usage(&mut m).unwrap().bytes, 150_010);
    // Durable: both entries survive a crash right after recycling.
    m.crash();
    assert_eq!(bin.usage(&mut m).unwrap().items, 2);
    // Restore recreates a missing parent; a taken name refuses.
    m.write_file("/users/1001/src/a.txt", b"new");
    assert_eq!(bin.restore(&mut m, a), Err(Error::Exists));
    m.remove("/users/1001/src/a.txt").unwrap();
    m.remove("/users/1001/src").unwrap();
    m.commit().unwrap();
    bin.restore(&mut m, a).unwrap();
    bin.restore(&mut m, t).unwrap();
    assert_eq!(m.read_file("/users/1001/src/a.txt"), b"alpha");
    assert_eq!(m.read_file("/users/1001/src/tree/sub/c.txt"), b"gamma");
    assert_eq!(bin.usage(&mut m).unwrap().items, 0);
    // Too big for the bin: the caller must confirm permanent deletion.
    let small = Bin {
        uid: U,
        limit: 100_000,
    };
    assert_eq!(
        small.recycle(&mut m, "/users/1001/src/tree").unwrap(),
        RecycleOutcome::TooBig
    );
    // Over the limit: the oldest entries are purged first.
    let RecycleOutcome::Recycled(x) = small.recycle(&mut m, "/users/1001/src/a.txt").unwrap()
    else {
        panic!()
    };
    m.write_file("/users/1001/src/big2.bin", &vec![1; 99_999]);
    m.clock += 10_000_000;
    m.commit().unwrap();
    let RecycleOutcome::Recycled(y) = small.recycle(&mut m, "/users/1001/src/big2.bin").unwrap()
    else {
        panic!()
    };
    let left: Vec<u64> = core::iter::successors(small.next(&mut m, 0).unwrap(), |i| {
        small.next(&mut m, i.id).unwrap()
    })
    .map(|i| i.id)
    .collect();
    assert_eq!(left, vec![y]);
    assert!(x < y);
    // Purge through a Delete operation: the record goes first.
    let (info, item) = small.entry_paths(&mut m, y).unwrap();
    let mut e = Box::new(Engine::new());
    e.begin(&mut m, 30, U, Mode::Delete, Policy::Ask, "")
        .unwrap();
    e.add(&mut m, 30, U, info.as_str()).unwrap();
    e.add(&mut m, 30, U, item.unwrap().as_str()).unwrap();
    assert_eq!(run(&mut e, &mut m, 30).state, State::Done);
    assert_eq!(small.usage(&mut m).unwrap().items, 0);
    // The bin itself and the home folder cannot be recycled.
    assert_eq!(
        small.recycle(&mut m, "/users/1001/$Recycle.Bin"),
        Err(Error::Invalid)
    );
    assert_eq!(small.recycle(&mut m, "/users/1001"), Err(Error::Invalid));
}
#[test]
fn windows_paths_resolve_to_their_stored_native_spelling() {
    let mut m = fixture();
    m.mkdir("/users/1001/AppData").unwrap();
    m.mkdir("/users/1001/AppData/Local").unwrap();
    m.write_file("/users/1001/AppData/Local/Probe.TXT", b"x");
    m.mkdir("/compat").unwrap();
    m.mkdir("/compat/Windows").unwrap();
    let native =
        crate::native_from_windows(&mut m, r"C:\USERS\1001\appdata\LOCAL\probe.txt").unwrap();
    assert_eq!(native.as_str(), "/users/1001/AppData/Local/Probe.TXT");
    let windows = crate::native_from_windows(&mut m, r"c:\windows").unwrap();
    assert_eq!(windows.as_str(), "/compat/Windows");
    assert_eq!(
        crate::native_from_windows(&mut m, r"C:\Users\1001\missing").err(),
        Some(Error::NotFound)
    );
    assert_eq!(
        crate::native_from_windows(&mut m, "/users/1001").err(),
        Some(Error::Invalid)
    );
    assert_eq!(
        crate::native_from_windows(&mut m, r"C:\Users\1001\..\1002").err(),
        Some(Error::Invalid)
    );
}
