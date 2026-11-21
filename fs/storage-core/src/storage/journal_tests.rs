extern crate std;
use super::*;
use std::{
    boxed::Box,
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    vec,
    vec::Vec,
};

const DISK_ID: [u8; 16] = *b"DoorP07DiskGUID0";
const HOME: Partition = Partition {
    type_guid: [1; 16],
    id: *b"DoorP07ExtGUID00",
    first: 8,
    sectors: 1024,
    attributes: 0,
};
const LOG: Partition = Partition {
    type_guid: *b"DoorRedoJournal1",
    id: *b"DoorP07LogGUID00",
    first: 1040,
    sectors: 528,
    attributes: 0,
};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Event {
    Read(u64, usize),
    Write(u64, usize),
    Flush,
}
struct Media {
    stable: Vec<u8>,
    volatile: Vec<u8>,
    events: Vec<Event>,
    fail_write: Option<(usize, usize)>,
    fail_flush: Option<usize>,
    persist_failed_flush: bool,
    fail_read: Option<usize>,
    writes: usize,
    flushes: usize,
    reads: usize,
}
#[derive(Clone)]
struct Device(Rc<RefCell<Media>>);
impl Device {
    fn new() -> Self {
        let mut bytes = vec![0; 1600 * 512];
        bytes[HOME.first as usize * 512..(HOME.first + HOME.sectors) as usize * 512].fill(0x11);
        for slot in 0..2 {
            header(
                &mut bytes[(LOG.first as usize + slot * 8) * 512..][..4096],
                0,
                0,
            );
        }
        Self(Rc::new(RefCell::new(Media {
            stable: bytes.clone(),
            volatile: bytes,
            events: Vec::new(),
            fail_write: None,
            fail_flush: None,
            persist_failed_flush: false,
            fail_read: None,
            writes: 0,
            flushes: 0,
            reads: 0,
        })))
    }
    fn crash(&self) {
        let mut m = self.0.borrow_mut();
        m.volatile = m.stable.clone();
        m.fail_write = None;
        m.fail_flush = None;
        m.fail_read = None;
        m.writes = 0;
        m.flushes = 0;
        m.reads = 0;
        m.events.clear();
    }
    fn mutate(&self, action: impl FnOnce(&mut [u8])) {
        let mut m = self.0.borrow_mut();
        action(&mut m.stable);
        m.volatile = m.stable.clone();
    }
    fn homes(&self) -> (u8, u8) {
        let m = self.0.borrow();
        let p = HOME.first as usize * 512;
        (m.stable[p], m.stable[p + 4096])
    }
    fn home_writes(&self) -> usize {
        self.0.borrow().events.iter().filter(|e|matches!(e,Event::Write(lba,_) if (HOME.first..HOME.first+HOME.sectors).contains(lba))).count()
    }
}
impl BlockDevice for Device {
    fn geometry(&self) -> Geometry {
        Geometry {
            sector_size: 512,
            sectors: 1600,
            read_only: false,
            flush_supported: true,
        }
    }
    fn read_sectors(&mut self, lba: u64, out: &mut [u8]) -> Result<()> {
        check_transfer(self.geometry(), lba, out.len())?;
        let mut m = self.0.borrow_mut();
        m.reads += 1;
        m.events.push(Event::Read(lba, out.len()));
        let start = lba as usize * 512;
        if m.fail_read == Some(m.reads) {
            out[..512].copy_from_slice(&m.volatile[start..start + 512]);
            return Err(Error::ShortIo);
        }
        out.copy_from_slice(&m.volatile[start..start + out.len()]);
        Ok(())
    }
    fn write_sectors(&mut self, lba: u64, input: &[u8]) -> Result<()> {
        check_transfer(self.geometry(), lba, input.len())?;
        let mut m = self.0.borrow_mut();
        m.writes += 1;
        m.events.push(Event::Write(lba, input.len()));
        let start = lba as usize * 512;
        if let Some((operation, prefix)) = m.fail_write
            && operation == m.writes
        {
            let n = prefix.min(input.len());
            m.volatile[start..start + n].copy_from_slice(&input[..n]);
            // Device writes may reach stable media even before a flush. Model a
            // genuinely persisted torn prefix, not merely discarded RAM writes.
            m.stable[start..start + n].copy_from_slice(&input[..n]);
            return Err(Error::ShortIo);
        }
        m.volatile[start..start + input.len()].copy_from_slice(input);
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        let mut m = self.0.borrow_mut();
        m.flushes += 1;
        m.events.push(Event::Flush);
        if m.fail_flush == Some(m.flushes) {
            if m.persist_failed_flush {
                m.stable = m.volatile.clone();
            }
            return Err(Error::Io);
        }
        m.stable = m.volatile.clone();
        Ok(())
    }
}
fn checksum(b: &mut [u8]) {
    put32(b, 120, 0);
    let c = crc32(b);
    put32(b, 120, c);
}
// Independent fixture encoder for the documented wire fields, not Journal::encode.
fn header(b: &mut [u8], state: u32, generation: u64) {
    b.fill(0);
    b[..8].copy_from_slice(b"DOORJNL1");
    for (p, v) in [(8, 1), (12, 4096), (16, state), (112, 512), (116, 4096)] {
        put32(b, p, v);
    }
    b[24..32].copy_from_slice(&generation.to_le_bytes());
    b[32..48].copy_from_slice(&DISK_ID);
    b[48..64].copy_from_slice(&HOME.id);
    b[64..80].copy_from_slice(&LOG.id);
    for (p, v) in [
        (80, HOME.first),
        (88, HOME.sectors),
        (96, LOG.first),
        (104, LOG.sectors),
    ] {
        b[p..p + 8].copy_from_slice(&v.to_le_bytes());
    }
    checksum(b);
}
fn prepare(j: &mut Journal<'_, Device>) {
    j.begin().unwrap();
    j.write_sectors(0, &[0x22; 4096]).unwrap();
    j.write_sectors(8, &[0x33; 4096]).unwrap();
    j.flush().unwrap();
}
fn interrupted(point: Checkpoint) -> Device {
    let device = Device::new();
    let mut work = Box::new(WorkingSet::new());
    {
        let (mut j, _) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
        prepare(&mut j);
        assert!(
            catch_unwind(AssertUnwindSafe(|| j.commit_with(|p| if p == point {
                panic!("simulated power cut")
            })))
            .is_err()
        );
    }
    device.crash();
    device
}

#[test]
fn journal_stages_overlay_without_durability_then_abort_discards() {
    let device = Device::new();
    let mut work = Box::new(WorkingSet::new());
    let (mut j, report) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
    assert_eq!(
        report,
        RecoveryReport {
            generation: 0,
            pages: 0,
            replayed: false,
            repaired_header: false
        }
    );
    assert!(core::mem::size_of_val(&j) < 512);
    assert_eq!(j.write_sectors(0, &[1; 512]), Err(Error::Busy));
    j.begin().unwrap();
    j.write_sectors(1, &[0x77; 512]).unwrap();
    j.flush().unwrap();
    let mut bytes = [0; 4096];
    j.read_sectors(0, &mut bytes).unwrap();
    assert_eq!(&bytes[..512], &[0x11; 512]);
    assert_eq!(&bytes[512..1024], &[0x77; 512]);
    assert_eq!(&bytes[1024..], &[0x11; 3072]);
    assert!(
        !device
            .0
            .borrow()
            .events
            .iter()
            .any(|e| matches!(e, Event::Write(..) | Event::Flush))
    );
    j.abort().unwrap();
    j.read_sectors(0, &mut bytes).unwrap();
    assert_eq!(bytes, [0x11; 4096]);
    j.begin().unwrap();
    let report = j.commit().unwrap();
    assert_eq!(report.pages, 0);
    assert_eq!(report.generation, 0);
}
#[test]
fn journal_flush_order_and_all_real_crash_boundaries_recover_atomically() {
    for point in [
        Checkpoint::PrepareFlushed,
        Checkpoint::CommitFlushed,
        Checkpoint::FirstHomeFlushed,
        Checkpoint::HomeFlushed,
        Checkpoint::CheckpointFlushed,
    ] {
        let device = interrupted(point);
        assert_eq!(
            device.homes(),
            match point {
                Checkpoint::PrepareFlushed | Checkpoint::CommitFlushed => (0x11, 0x11),
                Checkpoint::FirstHomeFlushed => (0x22, 0x11),
                _ => (0x22, 0x33),
            }
        );
        let mut work = Box::new(WorkingSet::new());
        let (j, report) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
        assert_eq!(
            report.generation,
            u64::from(point != Checkpoint::PrepareFlushed)
        );
        assert_eq!(
            report.replayed,
            matches!(
                point,
                Checkpoint::CommitFlushed | Checkpoint::FirstHomeFlushed | Checkpoint::HomeFlushed
            )
        );
        assert_eq!(
            device.homes(),
            if point == Checkpoint::PrepareFlushed {
                (0x11, 0x11)
            } else {
                (0x22, 0x33)
            }
        );
        drop(j);
        device.crash();
        let (_, second) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
        assert!(!second.replayed);
        assert_eq!(second.pages, 0);
        assert_eq!(device.home_writes(), 0);
    }
}
#[test]
fn journal_complete_commit_orders_payload_mirrors_home_and_checkpoints() {
    let device = Device::new();
    let mut work = Box::new(WorkingSet::new());
    let (mut j, _) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
    prepare(&mut j);
    device.0.borrow_mut().events.clear();
    let mut points = Vec::new();
    let report = j.commit_with(|p| points.push(p)).unwrap();
    assert_eq!(
        report,
        CommitReport {
            generation: 1,
            pages: 2
        }
    );
    assert_eq!(
        points,
        [
            Checkpoint::PrepareFlushed,
            Checkpoint::CommitFlushed,
            Checkpoint::FirstHomeFlushed,
            Checkpoint::HomeFlushed,
            Checkpoint::CheckpointFlushed
        ]
    );
    assert_eq!(
        device.0.borrow().events,
        [
            Event::Read(HOME.first, 4096),
            Event::Read(HOME.first + 8, 4096),
            Event::Write(LOG.first + 16, 4096),
            Event::Write(LOG.first + 24, 4096),
            Event::Flush,
            Event::Write(LOG.first, 4096),
            Event::Flush,
            Event::Write(LOG.first + 8, 4096),
            Event::Flush,
            Event::Write(HOME.first, 4096),
            Event::Flush,
            Event::Write(HOME.first + 8, 4096),
            Event::Flush,
            Event::Write(LOG.first, 4096),
            Event::Flush,
            Event::Write(LOG.first + 8, 4096),
            Event::Flush
        ]
    );
}
#[test]
fn journal_full_working_set_rejects_whole_request_without_partial_overlay_change() {
    let device = Device::new();
    let mut work = Box::new(WorkingSet::new());
    let (mut j, _) = Journal::open(device, HOME, LOG, DISK_ID, &mut work).unwrap();
    j.begin().unwrap();
    for page in 0..64 {
        j.write_sectors(page * 8, &[0x44; 4096]).unwrap();
    }
    assert_eq!(j.staged_pages(), 64);
    assert_eq!(j.write_sectors(63 * 8, &[0x99; 8192]), Err(Error::NoSpace));
    let mut bytes = [0; 8192];
    j.read_sectors(63 * 8, &mut bytes).unwrap();
    assert_eq!(&bytes[..4096], &[0x44; 4096]);
    assert_eq!(&bytes[4096..], &[0x11; 4096]);
    assert!(!j.is_poisoned());
    j.abort().unwrap();
    assert_eq!(j.staged_pages(), 0);
}
#[test]
fn journal_torn_write_prefixes_and_flush_failures_reopen_old_or_complete_new() {
    for operation in 1..=8 {
        for prefix in [0, 1, 511, 512, 1024, 4095, 4096] {
            let device = Device::new();
            let mut work = Box::new(WorkingSet::new());
            {
                let (mut j, _) =
                    Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
                prepare(&mut j);
                device.0.borrow_mut().fail_write = Some((operation, prefix));
                assert_eq!(j.commit(), Err(Error::ShortIo));
                assert!(j.is_poisoned());
                assert_eq!(j.abort(), Err(Error::Poisoned));
                assert_eq!(j.begin(), Err(Error::Poisoned));
            }
            device.crash();
            Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
            assert!(
                matches!(device.homes(), (0x11, 0x11) | (0x22, 0x33)),
                "write {operation} prefix {prefix}"
            );
        }
    }
    for persisted in [false, true] {
        for operation in 1..=7 {
            let device = Device::new();
            let mut work = Box::new(WorkingSet::new());
            {
                let (mut j, _) =
                    Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
                prepare(&mut j);
                device.0.borrow_mut().fail_flush = Some(operation);
                device.0.borrow_mut().persist_failed_flush = persisted;
                assert_eq!(j.commit(), Err(Error::Io));
                assert!(j.is_poisoned());
            }
            device.crash();
            Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
            assert!(matches!(device.homes(), (0x11, 0x11) | (0x22, 0x33)));
        }
    }
}
#[test]
fn journal_validates_every_record_and_payload_before_any_replay_write() {
    for mutation in 0..8 {
        let device = interrupted(Checkpoint::CommitFlushed);
        device.mutate(|bytes| {
            if mutation == 0 {
                bytes[(LOG.first as usize + 24) * 512 + 4095] ^= 1;
                return;
            }
            for slot in 0..2 {
                let b = &mut bytes[(LOG.first as usize + slot * 8) * 512..][..4096];
                match mutation {
                    1 => put32(b, 136, 0),
                    2 => put32(b, 136, 128),
                    3 => b[32] ^= 1,
                    4 => put32(b, 20, 65),
                    5 => b[200] = 1,
                    6 => put32(b, 112, 4096),
                    7 => b[124] = 1,
                    _ => unreachable!(),
                }
                checksum(b);
            }
        });
        let mut work = Box::new(WorkingSet::new());
        assert!(matches!(
            Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work),
            Err(Error::Corrupt)
        ));
        assert_eq!(device.home_writes(), 0);
        assert_eq!(device.homes(), (0x11, 0x11));
    }
}
#[test]
fn journal_mirror_repair_precedes_payload_reuse_and_preserves_generation() {
    let device = Device::new();
    let mut work = Box::new(WorkingSet::new());
    {
        let (mut j, _) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
        prepare(&mut j);
        device.0.borrow_mut().fail_write = Some((8, 0));
        assert_eq!(j.commit(), Err(Error::ShortIo));
    }
    device.crash();
    {
        let (mut j, report) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
        assert!(report.repaired_header);
        assert!(!report.replayed);
        assert_eq!(report.generation, 1);
        j.begin().unwrap();
        j.write_sectors(0, &[0x66; 4096]).unwrap();
        assert!(
            catch_unwind(AssertUnwindSafe(|| j.commit_with(|p| {
                if p == Checkpoint::PrepareFlushed {
                    panic!("cut")
                }
            })))
            .is_err()
        );
    }
    device.crash();
    let (_, report) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
    assert_eq!(report.generation, 1);
    assert!(!report.replayed);
    assert_eq!(device.homes(), (0x22, 0x33));
}
#[test]
fn journal_torn_mirror_fallback_generation_limits_and_blank_media_fail_closed() {
    let device = interrupted(Checkpoint::CommitFlushed);
    device.mutate(|b| b[LOG.first as usize * 512] ^= 1);
    let mut work = Box::new(WorkingSet::new());
    let (_, report) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
    assert!(report.replayed && report.repaired_header);
    device.mutate(|bytes| {
        for slot in 0..2 {
            header(
                &mut bytes[(LOG.first as usize + slot * 8) * 512..][..4096],
                0,
                u64::MAX,
            );
        }
    });
    {
        let (mut j, _) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
        assert_eq!(j.begin(), Err(Error::NoSpace));
    }
    device.mutate(|b| b[LOG.first as usize * 512..(LOG.first as usize + 16) * 512].fill(0));
    assert!(matches!(
        Journal::open(device, HOME, LOG, DISK_ID, &mut work),
        Err(Error::Corrupt)
    ));
}
#[test]
fn journal_short_reads_poison_staging_and_recovery_does_not_write() {
    let device = Device::new();
    let mut work = Box::new(WorkingSet::new());
    {
        let (mut j, _) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
        j.begin().unwrap();
        device.0.borrow_mut().fail_read = Some(3);
        assert_eq!(j.write_sectors(0, &[2; 512]), Err(Error::ShortIo));
        assert!(j.is_poisoned());
        assert_eq!(j.commit(), Err(Error::Poisoned));
    }
    device.crash();
    device.0.borrow_mut().fail_read = Some(2);
    assert!(matches!(
        Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work),
        Err(Error::ShortIo)
    ));
    assert_eq!(device.home_writes(), 0);
}

#[test]
fn journal_recovery_itself_can_be_interrupted_and_repeated() {
    for operation in 1..=4 {
        for prefix in [0, 512, 4095, 4096] {
            let device = interrupted(Checkpoint::CommitFlushed);
            let mut work = Box::new(WorkingSet::new());
            device.0.borrow_mut().fail_write = Some((operation, prefix));
            assert!(matches!(
                Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work),
                Err(Error::ShortIo)
            ));
            device.crash();
            Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
            assert_eq!(device.homes(), (0x22, 0x33));
        }
    }
    for operation in 1..=4 {
        let device = interrupted(Checkpoint::CommitFlushed);
        let mut work = Box::new(WorkingSet::new());
        device.0.borrow_mut().fail_flush = Some(operation);
        assert!(matches!(
            Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work),
            Err(Error::Io)
        ));
        device.crash();
        Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
        assert_eq!(device.homes(), (0x22, 0x33));
    }
}

#[test]
fn journal_repair_preserves_the_only_valid_header_in_either_slot() {
    for selected in 0..2 {
        for prefix in [0, 512, 4095] {
            let device = Device::new();
            device.mutate(|b| b[(LOG.first as usize + (1 - selected) * 8) * 512] ^= 1);
            device.0.borrow_mut().fail_write = Some((1, prefix));
            let mut work = Box::new(WorkingSet::new());
            assert!(matches!(
                Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work),
                Err(Error::ShortIo)
            ));
            device.crash();
            let (_, report) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
            assert!(!report.replayed);
            assert_eq!(report.generation, 0);
            assert_eq!(device.homes(), (0x11, 0x11));
        }
    }
}

#[test]
fn journal_checksum_valid_conflicting_mirrors_are_not_treated_as_torn_writes() {
    for mutation in 0..4 {
        let device = interrupted(Checkpoint::CommitFlushed);
        device.mutate(|bytes| {
            let b = &mut bytes[LOG.first as usize * 512..][..4096];
            match mutation {
                0 => put32(b, 128, 2), // Different valid target in same generation.
                1 => b[48] ^= 1,       // Wrong home volume identity.
                2 => b[24..32].copy_from_slice(&3u64.to_le_bytes()),
                3 => b[24..32].copy_from_slice(&0u64.to_le_bytes()),
                _ => unreachable!(),
            }
            checksum(b);
        });
        let mut work = Box::new(WorkingSet::new());
        assert!(matches!(
            Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work),
            Err(Error::Corrupt)
        ));
        assert_eq!(device.home_writes(), 0);
    }
}

#[test]
fn journal_last_payload_short_read_cannot_replay_the_valid_first_page() {
    let device = interrupted(Checkpoint::CommitFlushed);
    device.0.borrow_mut().fail_read = Some(4);
    let mut work = Box::new(WorkingSet::new());
    assert!(matches!(
        Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work),
        Err(Error::ShortIo)
    ));
    assert_eq!(device.home_writes(), 0);
    assert_eq!(device.homes(), (0x11, 0x11));
}

#[test]
fn journal_compacts_unchanged_pages_before_publishing_and_preserves_changed_order() {
    let device = Device::new();
    let mut work = Box::new(WorkingSet::new());
    let (mut journal, _) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
    journal.begin().unwrap();
    // Model ext2's first dirty superblock returning to its original clean bytes.
    journal.write_sectors(0, &[0xff; 4096]).unwrap();
    journal.write_sectors(24, &[0x33; 4096]).unwrap();
    journal.write_sectors(16, &[0x11; 4096]).unwrap();
    journal.write_sectors(8, &[0x22; 4096]).unwrap();
    journal.write_sectors(0, &[0x11; 4096]).unwrap();
    device.0.borrow_mut().events.clear();
    let report = journal
        .commit_with(|point| {
            let media = device.0.borrow();
            if point == Checkpoint::CommitFlushed {
                let header = &media.stable[LOG.first as usize * 512..][..4096];
                assert_eq!(u32le(header, 20), 2);
                assert_eq!(u32le(header, 128), 3);
                assert_eq!(u32le(header, 136), 1);
                assert_eq!(u32le(header, 132), crc32(&[0x33; 4096]));
                assert_eq!(u32le(header, 140), crc32(&[0x22; 4096]));
            }
            if point == Checkpoint::FirstHomeFlushed {
                let home = &media.stable[HOME.first as usize * 512..][..16384];
                assert_eq!(&home[..12288], &[0x11; 12288]);
                assert_eq!(&home[12288..], &[0x33; 4096]);
            }
        })
        .unwrap();
    assert_eq!(
        report,
        CommitReport {
            generation: 1,
            pages: 2
        }
    );
    let media = device.0.borrow();
    assert_eq!(
        &media.events[..4],
        &[
            Event::Read(HOME.first, 4096),
            Event::Read(HOME.first + 24, 4096),
            Event::Read(HOME.first + 16, 4096),
            Event::Read(HOME.first + 8, 4096)
        ]
    );
    assert!(matches!(media.events[4], Event::Write(..)));
    assert!(
        !media.events.iter().any(
            |e| matches!(e, Event::Write(lba,_) if *lba == HOME.first || *lba == HOME.first+16)
        )
    );
    assert_eq!(
        &media.stable[(HOME.first as usize + 8) * 512..][..4096],
        &[0x22; 4096]
    );
}

#[test]
fn journal_all_unchanged_commit_performs_no_writes_or_flushes_or_generation_advance() {
    let device = Device::new();
    let mut work = Box::new(WorkingSet::new());
    let (mut journal, _) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
    prepare(&mut journal);
    journal.commit().unwrap();
    journal.begin().unwrap();
    journal.write_sectors(0, &[0x99; 4096]).unwrap();
    journal.write_sectors(0, &[0x22; 4096]).unwrap();
    journal.write_sectors(8, &[0x33; 4096]).unwrap();
    let before = device.0.borrow().stable.clone();
    device.0.borrow_mut().events.clear();
    let report = journal
        .commit_with(|_| panic!("unchanged transaction published a checkpoint"))
        .unwrap();
    assert_eq!(
        report,
        CommitReport {
            generation: 1,
            pages: 0
        }
    );
    assert!(!journal.active());
    assert!(!journal.is_poisoned());
    assert_eq!(journal.staged_pages(), 0);
    assert_eq!(
        device.0.borrow().events,
        [
            Event::Read(HOME.first, 4096),
            Event::Read(HOME.first + 8, 4096)
        ]
    );
    assert_eq!(device.0.borrow().stable, before);
    journal.begin().unwrap();
    journal.write_sectors(16, &[0x44; 4096]).unwrap();
    assert_eq!(journal.commit().unwrap().generation, 2);
}

#[test]
fn journal_comparison_short_read_poison_preserves_both_log_and_home() {
    let device = Device::new();
    let mut work = Box::new(WorkingSet::new());
    {
        let (mut journal, _) =
            Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
        journal.begin().unwrap();
        // Compaction has already moved one changed page when the final read
        // fails. That partially compacted RAM must never be published or reused.
        journal.write_sectors(0, &[0x11; 4096]).unwrap();
        journal.write_sectors(8, &[0x22; 4096]).unwrap();
        journal.write_sectors(16, &[0x33; 4096]).unwrap();
        let before = device.0.borrow().stable.clone();
        let next = device.0.borrow().reads + 3;
        device.0.borrow_mut().fail_read = Some(next);
        device.0.borrow_mut().events.clear();
        assert_eq!(journal.commit(), Err(Error::ShortIo));
        assert!(journal.is_poisoned());
        assert_eq!(journal.abort(), Err(Error::Poisoned));
        assert_eq!(journal.begin(), Err(Error::Poisoned));
        assert_eq!(device.0.borrow().stable, before);
        assert!(
            !device
                .0
                .borrow()
                .events
                .iter()
                .any(|e| matches!(e, Event::Write(..) | Event::Flush))
        );
    }
    device.crash();
    let (_, report) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
    assert_eq!(report.generation, 0);
    assert!(!report.replayed);
    assert_eq!(device.homes(), (0x11, 0x11));
}
#[test]
fn journal_reads_each_unstaged_run_as_one_home_transfer() {
    // Regression: per-sector forwarding made each 4 KiB block eight device
    // requests, so Init's 192-byte executable capture timed out in the guest.
    let device = Device::new();
    let mut work = Box::new(WorkingSet::new());
    let (mut j, _) = Journal::open(device.clone(), HOME, LOG, DISK_ID, &mut work).unwrap();
    let since = |mark: usize| device.0.borrow().events[mark..].to_vec();
    let mark = device.0.borrow().events.len();
    let mut block = [0; 4096];
    j.read_sectors(8, &mut block).unwrap();
    assert_eq!(block, [0x11; 4096]);
    assert_eq!(since(mark), [Event::Read(HOME.first + 8, 4096)]);
    // A staged middle page is served from the overlay and splits the run.
    j.begin().unwrap();
    j.write_sectors(9, &[0x44; 512]).unwrap();
    let mark = device.0.borrow().events.len();
    let mut pages = [0; 3 * 4096];
    j.read_sectors(0, &mut pages).unwrap();
    assert_eq!(
        since(mark),
        [
            Event::Read(HOME.first, 4096),
            Event::Read(HOME.first + 16, 4096)
        ]
    );
    assert_eq!(&pages[..4608], &[0x11; 4608]);
    assert_eq!(&pages[4608..5120], &[0x44; 512]);
    assert_eq!(&pages[5120..], &[0x11; 3 * 4096 - 5120]);
    j.abort().unwrap();
}
