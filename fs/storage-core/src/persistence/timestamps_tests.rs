//! Catalog v4/v5 timestamps: what each mutation records, durability through
//! the redo journal, and lossless reading of untimed v1/v3 catalogs.
use super::*;
use crate::storage::{Journal, WorkingSet};

const T0: u64 = 134_774 * 86_400 * 10_000_000 + 1_792_413_059 * 10_000_000;
const T1: u64 = T0 + 10_000_000;
const T2: u64 = T1 + 10_000_000;
const T3: u64 = T2 + 10_000_000;
const T4: u64 = T3 + 10_000_000;

fn times(v: &mut PersistentVolume<'_, impl Vfs>, path: &str) -> FileTimes {
    let subject = if path.starts_with("/users/1001") {
        user(1001)
    } else {
        system()
    };
    v.metadata(subject, Path::Native(path), Context::root())
        .unwrap()
        .times
}

#[test]
fn mutations_record_windows_times_and_survive_journal_remount() {
    let (home, log) = partitions();
    let mut disk = journal_disk();
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    {
        let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
        journal.begin().unwrap();
        let mut v = PersistentVolume::initialize(
            Ext2::mount(journal).unwrap(),
            &mut state,
            &mut catalog,
            system(),
        )
        .unwrap();
        // Baseline folders were created without a clock: not recorded.
        assert_eq!(times(&mut v, "/users"), FileTimes::NONE);
        v.set_clock(T0);
        v.mkdir(
            system(),
            Path::Native("/users/1001"),
            Context::root(),
            Some(Acl::owner_full(1001)),
        )
        .unwrap();
        assert_eq!(times(&mut v, "/users/1001"), FileTimes::at(T0));
        assert_eq!(
            times(&mut v, "/users"),
            FileTimes {
                modified: T0,
                ..FileTimes::NONE
            }
        );
        v.set_clock(T1);
        let (file, created) = v
            .open(
                user(1001),
                Path::Native("/users/1001/a.txt"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        assert!(created);
        assert_eq!(v.info(&file).unwrap().times, FileTimes::at(T1));
        assert_eq!(
            times(&mut v, "/users/1001"),
            FileTimes {
                modified: T1,
                ..FileTimes::at(T0)
            }
        );
        v.set_clock(T2);
        v.write(&file, 0, b"door").unwrap();
        let written = FileTimes {
            created: T1,
            modified: T2,
            accessed: T2,
        };
        assert_eq!(v.info(&file).unwrap().times, written);
        // Reads never update the access time (ACCESS_UPDATES is off).
        const { assert!(!ACCESS_UPDATES) };
        v.set_clock(T3);
        let mut out = [0; 4];
        assert_eq!(v.read(&file, 0, &mut out), Ok(4));
        assert_eq!(v.info(&file).unwrap().times, written);
        v.close(file).unwrap();
        // A rename keeps the item's times and touches only its folder.
        v.set_clock(T4);
        v.rename_path(
            user(1001),
            Path::Native("/users/1001/a.txt"),
            Path::Native("/users/1001/b.txt"),
            Context::root(),
        )
        .unwrap();
        assert_eq!(times(&mut v, "/users/1001/b.txt"), written);
        assert_eq!(times(&mut v, "/users/1001").modified, T4);
        // A second folder whose content changes while no clock exists.
        v.mkdir(
            user(1001),
            Path::Native("/users/1001/c"),
            Context::root(),
            None,
        )
        .unwrap();
        let (file, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/c/d.txt"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        v.set_clock(0);
        v.write(&file, 0, b"x").unwrap();
        // No clock: the last known times are kept, never erased to 0.
        assert_eq!(v.info(&file).unwrap().times, FileTimes::at(T4));
        assert_eq!(times(&mut v, "/users/1001/c").modified, T4);
        v.close(file).unwrap();
        v.set_clock(u64::MAX); // out of FILETIME range: treated as unavailable
        v.mkdir(
            user(1001),
            Path::Native("/users/1001/e"),
            Context::root(),
            None,
        )
        .unwrap();
        assert_eq!(times(&mut v, "/users/1001/e"), FileTimes::NONE);
        v.commit().unwrap();
    }
    let (mut journal, report) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
    assert_eq!(report.pages, 0);
    journal.begin().unwrap();
    let mut state = State::boxed();
    let mut v =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    assert_eq!(
        times(&mut v, "/users/1001/b.txt"),
        FileTimes {
            created: T1,
            modified: T2,
            accessed: T2
        }
    );
    assert_eq!(times(&mut v, "/users/1001/c/d.txt"), FileTimes::at(T4));
    // The directory listing carries the same times.
    let entry = v
        .list(user(1001), Path::Native("/users/1001"), Context::root(), 0)
        .unwrap()
        .unwrap();
    assert_eq!(
        entry.info.times,
        FileTimes {
            created: T1,
            modified: T2,
            accessed: T2
        }
    );
    // Wire: version 4, created/modified in node bytes 208..224, accessed table.
    let mut wire = Box::new([0; CATALOG_BYTES]);
    codec::encode(v.state, &mut wire[..]).unwrap();
    assert_eq!(u32::from_le_bytes(wire[8..12].try_into().unwrap()), 4);
    let index = v
        .state
        .nodes
        .iter()
        .position(|n| n.name.units() == wide("b.txt"))
        .unwrap();
    let node = 128 + index * 224;
    assert_eq!(
        u64::from_le_bytes(wire[node + 208..node + 216].try_into().unwrap()),
        T1
    );
    assert_eq!(
        u64::from_le_bytes(wire[node + 216..node + 224].try_into().unwrap()),
        T2
    );
    let accessed = 15488 + index * 8;
    assert_eq!(
        u64::from_le_bytes(wire[accessed..accessed + 8].try_into().unwrap()),
        T2
    );
}

fn reseal(wire: &mut [u8]) {
    let crc = codec::checksum(wire);
    wire[40..44].copy_from_slice(&crc.to_le_bytes());
}

#[test]
fn untimed_catalogs_read_as_not_recorded_and_timed_fields_are_validated() {
    with_volume(|v| {
        v.set_clock(T0);
        v.mkdir(system(), Path::Native("/tmp/x"), Context::root(), None)
            .unwrap();
        let mut timed = Box::new([0; CATALOG_BYTES]);
        codec::encode(v.state, &mut timed[..]).unwrap();
        let index = v
            .state
            .nodes
            .iter()
            .position(|n| n.name.units() == wide("x"))
            .unwrap();
        let node = 128 + index * 224;
        let mut target = State::boxed();
        // A catalog v1 image (written before timestamps): every time reads
        // as not recorded, and no migration write is forced.
        let mut v1 = timed.clone();
        v1[8..12].copy_from_slice(&1u32.to_le_bytes());
        for i in 0..32 {
            v1[128 + i * 224 + 208..128 + i * 224 + 224].fill(0);
        }
        v1[15488..15488 + 256].fill(0);
        reseal(&mut v1[..]);
        assert_eq!(codec::decode(&v1[..], &mut target), Ok(false));
        assert!(target.nodes.iter().all(|n| n.times == FileTimes::NONE));
        // ... which re-encodes as v4.
        let mut upgraded = Box::new([0; CATALOG_BYTES]);
        codec::encode(&target, &mut upgraded[..]).unwrap();
        assert_eq!(u32::from_le_bytes(upgraded[8..12].try_into().unwrap()), 4);
        // v1 keeps the time fields reserved.
        for offset in [node + 208, node + 216, 15488 + index * 8] {
            let mut bad = v1.clone();
            bad[offset] = 1;
            reseal(&mut bad[..]);
            assert_eq!(
                codec::decode(&bad[..], &mut target),
                Err(Error::Corrupt),
                "{offset}"
            );
        }
        // v4: out-of-range values, an access time for an empty slot and the
        // remaining reserved tail are rejected; unknown versions are unsupported.
        let empty = v.state.nodes[..32].iter().position(|n| n.id == 0).unwrap();
        for (offset, value) in [
            (node + 208, u64::MAX),
            (node + 216, 1 << 63),
            (15488 + index * 8, 1 << 63),
            (15488 + empty * 8, 1),
            (15744, 1),
        ] {
            let mut bad = timed.clone();
            bad[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
            reseal(&mut bad[..]);
            assert_eq!(
                codec::decode(&bad[..], &mut target),
                Err(Error::Corrupt),
                "{offset}"
            );
        }
        for version in [2u32, 7] {
            let mut bad = timed.clone();
            bad[8..12].copy_from_slice(&version.to_le_bytes());
            reseal(&mut bad[..]);
            assert_eq!(
                codec::decode(&bad[..], &mut target),
                Err(Error::Unsupported)
            );
        }
        assert_eq!(codec::decode(&timed[..], &mut target), Ok(false));
        assert_eq!(target.nodes[index].times, FileTimes::at(T0));
    });
}

#[test]
fn production_catalog_v5_places_high_node_access_times_after_the_node_area() {
    let mut state = State::boxed_production();
    let mut buffer = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
    let mut volume = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    for index in 0..20 {
        volume.set_clock(T0 + index);
        volume
            .mkdir(
                system(),
                Path::Native(&std::format!("/tmp/d{index}")),
                Context::root(),
                None,
            )
            .unwrap();
    }
    let index = volume
        .state
        .nodes
        .iter()
        .position(|n| n.name.units() == wide("d19"))
        .unwrap();
    assert!(index >= 32);
    let mut wire = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    codec::encode(volume.state, &mut wire[..]).unwrap();
    assert_eq!(u32::from_le_bytes(wire[8..12].try_into().unwrap()), 5);
    let mut target = State::boxed();
    assert_eq!(codec::decode(&wire[..], &mut target), Ok(false));
    assert_eq!(target.nodes[index].times, FileTimes::at(T0 + 19));
    // A v3 image of the same catalog has no times and reads as not recorded.
    let mut v3 = wire.clone();
    v3[8..12].copy_from_slice(&3u32.to_le_bytes());
    for i in 0..256 {
        let node = if i < 32 {
            128 + i * 224
        } else {
            CATALOG_BYTES + (i - 32) * 224
        };
        v3[node + 208..node + 224].fill(0);
    }
    v3[15488..15744].fill(0);
    v3[66560..68352].fill(0);
    reseal(&mut v3[..]);
    assert_eq!(codec::decode(&v3[..], &mut target), Ok(false));
    assert!(target.nodes.iter().all(|n| n.times == FileTimes::NONE));
    let mut bad = wire.clone();
    bad[68352] = 1; // reserved tail after the access table
    reseal(&mut bad[..]);
    assert_eq!(codec::decode(&bad[..], &mut target), Err(Error::Corrupt));
}

#[test]
fn time_zone_and_regional_format_are_checked_per_user_settings() {
    with_volume(|v| {
        // Defaults: UTC (the RTC's base) and English (United States).
        assert_eq!(v.setting_get(user(1001), 1001, Setting::TimeZone), Ok(16));
        assert_eq!(
            v.setting_get(user(1001), 1001, Setting::RegionalFormat),
            Ok(1)
        );
        v.setting_set(user(1001), 1001, Setting::TimeZone, 3)
            .unwrap();
        v.setting_set(user(1001), 1001, Setting::RegionalFormat, 3)
            .unwrap();
        for (setting, bad) in [
            (Setting::TimeZone, 0),
            (Setting::TimeZone, 50),
            (Setting::RegionalFormat, 8),
        ] {
            assert_eq!(
                v.setting_set(user(1001), 1001, setting, bad),
                Err(Error::InvalidParameter)
            );
        }
        assert_eq!(
            v.setting_set(user(1002), 1001, Setting::TimeZone, 1),
            Err(Error::AccessDenied)
        );
        assert_eq!(v.setting_get(user(1002), 1002, Setting::TimeZone), Ok(16));
        let mut wire = Box::new([0; CATALOG_BYTES]);
        codec::encode(v.state, &mut wire[..]).unwrap();
        let mut target = State::boxed();
        codec::decode(&wire[..], &mut target).unwrap();
        v.state.config.values = target.config.values;
        assert_eq!(v.setting_get(system(), 1001, Setting::TimeZone), Ok(3));
        assert_eq!(
            v.setting_get(system(), 1001, Setting::RegionalFormat),
            Ok(3)
        );
    });
}
