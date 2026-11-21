use super::*;
use crate::storage::audit::crc32;
use std::collections::BTreeMap;
fn put(b: &mut [u8], at: usize, value: u64, width: usize) {
    b[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
}
fn checksum(b: &mut [u8], at: usize) {
    b[at..at + 4].fill(0);
    let crc = crc32(b);
    put(b, at, u64::from(crc), 4);
}
#[test]
fn p07_formatter_preserves_p06_and_validates_both_capacities() {
    let p06 = crate::storage::format::image();
    for tiny in [false, true] {
        let bytes = format::image(tiny);
        gpt(&bytes).unwrap();
        crate::storage::audit::fat(&bytes).unwrap();
        let j = journal::inspect(&bytes).unwrap();
        assert!(!j.selected().committed);
        assert_eq!(j.selected().generation, 0);
        let t = tree::inspect(&j.replayed_home(&bytes).unwrap()).unwrap();
        assert_eq!(t.free_blocks, if tiny { 22 } else { 8182 });
        assert_eq!(catalog::inspect(&t).unwrap()["state"], "initial");
        assert_eq!(
            bytes[2048 * 512..133120 * 512],
            p06[2048 * 512..133120 * 512]
        );
    }
    crate::storage::audit::inspect(&p06, false).unwrap();
}
#[test]
fn independent_journal_rejects_corrupt_identity_targets_payload_and_conflicting_copies() {
    let mut bytes = format::image(false);
    let base = LOG_FIRST as usize * 512;
    for i in 0..2 {
        let h = &mut bytes[base + i * 4096..base + (i + 1) * 4096];
        put(h, 16, 1, 4);
        put(h, 20, 2, 4);
        put(h, 24, 1, 8);
        put(h, 128, 9, 4);
        put(h, 132, u64::from(crc32(&[0; 4096])), 4);
        put(h, 136, 10, 4);
        put(h, 140, u64::from(crc32(&[0; 4096])), 4);
        checksum(h, 120);
    }
    assert_eq!(
        journal::inspect(&bytes).unwrap().selected().entries.len(),
        2
    );
    for (at, value, width) in [
        (32, 1, 1),
        (20, 65, 4),
        (128, 8192, 4),
        (136, 9, 4),
        (124, 1, 1),
        (144, 1, 1),
    ] {
        let saved = bytes[base..base + 4096].to_vec();
        put(&mut bytes, base + at, value, width);
        checksum(&mut bytes[base..base + 4096], 120);
        assert!(journal::inspect(&bytes).is_err());
        bytes[base..base + 4096].copy_from_slice(&saved);
    }
    bytes[base + 8192] ^= 1;
    assert!(journal::inspect(&bytes).is_err());
    bytes[base + 8192] ^= 1;
    put(&mut bytes, base + 128, 11, 4);
    checksum(&mut bytes[base..base + 4096], 120);
    assert!(journal::inspect(&bytes).is_err());
}

#[test]
fn independent_journal_accepts_only_publishable_mirror_relations_in_either_order() {
    let mut bytes = format::image(false);
    let base = LOG_FIRST as usize * 512;
    let fresh = bytes[base..base + 4096].to_vec();
    let payload_crc = u64::from(crc32(&[0; 4096]));
    let header = |generation, committed: bool, block| {
        let mut h = fresh.clone();
        put(&mut h, 24, generation, 8);
        if committed {
            put(&mut h, 16, 1, 4);
            put(&mut h, 20, 1, 4);
            put(&mut h, 128, block, 4);
            put(&mut h, 132, payload_crc, 4);
        }
        checksum(&mut h, 120);
        h
    };
    for (a, b, accepted) in [
        ((0, false, 0), (0, false, 0), true),
        ((7, false, 0), (7, false, 0), true),
        ((7, true, 9), (7, true, 9), true),
        ((7, true, 9), (7, false, 0), true),
        ((0, false, 0), (1, true, 9), true),
        ((7, false, 0), (8, true, 9), true),
        ((u64::MAX - 1, false, 0), (u64::MAX, true, 9), true),
        ((u64::MAX, true, 9), (u64::MAX, false, 0), true),
        ((0, false, 0), (2, false, 0), false),
        ((1, false, 0), (0, false, 0), false),
        ((9, false, 0), (0, false, 0), false),
        ((0, false, 0), (2, true, 9), false),
        ((7, true, 9), (8, true, 9), false),
        ((7, true, 9), (8, false, 0), false),
        ((7, true, 9), (7, true, 10), false),
        ((0, true, 9), (0, false, 0), false),
        ((u64::MAX, false, 0), (1, true, 9), false),
    ] {
        for pair in [[a, b], [b, a]] {
            for (slot, (generation, committed, block)) in pair.into_iter().enumerate() {
                bytes[base + slot * 4096..base + (slot + 1) * 4096]
                    .copy_from_slice(&header(generation, committed, block));
            }
            let result = journal::inspect(&bytes);
            assert_eq!(result.is_ok(), accepted, "mirror pair {pair:?}");
            if let Ok(log) = result {
                assert_eq!(log.selected().generation, a.0.max(b.0));
                assert_eq!(log.selected().committed, a.0 != b.0 || (a.1 && b.1));
            }
        }
    }
}
#[test]
fn p07_schedule_and_interruption_reject_weakened_or_unbound_claims() {
    let old = crate::scenarios::cases("P06").unwrap();
    let all = crate::scenarios::cases("P07").unwrap();
    assert_eq!(all.len(), 35);
    for (a, b) in old.iter().zip(&all) {
        assert_eq!(
            (a.id, a.mode, a.profile, &a.markers),
            (b.id, b.mode, b.profile, &b.markers)
        );
    }
    let good = "DOOR_TEST v1 case POST_EXIT_BOOT_SERVICES\nDOOR_P07 INTERRUPT stage=first_home generation=2\nDOOR_TEST v1 case JOURNAL_INTERRUPTED\n";
    assert!(interrupted(good, "case", "journal-first-home"));
    for bad in [
        good.replace("case", "other"),
        good.replace("generation=2", "generation=02"),
        good.replace("first_home", "home"),
        good.replace("generation=2", "generation=0"),
        format!("{good}DOOR_TEST v1 case KERNEL_DIAGNOSTIC PASS\n"),
        format!("{good}DOOR_P07 INTERRUPT stage=first_home generation=2\n"),
    ] {
        assert!(!interrupted(&bad, "case", "journal-first-home"));
    }
    let case = &all[17];
    let lines = case
        .markers
        .iter()
        .filter(|m| m.starts_with("DOOR_P07 "))
        .map(|m| {
            if m.starts_with("DOOR_P07 STACK") {
                format!("{m} sampled_remaining=32768 stack_bytes=131072")
            } else if m.starts_with("DOOR_P07 RECOVERY") {
                format!("{m} generation=0")
            } else {
                (*m).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(scenarios::serial_matches(&case.markers, &lines));
    for bad in [
        lines.replace("acl=1", "acl=0"),
        lines.replace("crc32=01ea357b", "crc32=69f67483"),
        lines.replace("replace=1", "replace=0"),
        lines.replace("generation=0", "generation=00"),
        lines.replace("sampled_remaining=32768", "sampled_remaining=16383"),
    ] {
        assert!(!scenarios::serial_matches(&case.markers, &bad));
    }
    assert!(crate::scenarios::cases("P18").is_err());
    let markers = ["DOOR_P07 CAPACITY PASS"];
    let full = "DOOR_P07 CAPACITY PASS files=9 free_blocks=0 rollback=1 preserved=1";
    assert!(scenarios::serial_matches(&markers, full));
    for bad in [
        full.replace("files=9", "files=8"),
        full.replace("files=9", "files=09"),
        full.replace("free_blocks=0", "free_blocks=1"),
    ] {
        assert!(!scenarios::serial_matches(&markers, &bad));
    }
}
pub(crate) fn catalog_tree(new: bool) -> tree::Tree {
    let mut b = vec![0; 16384];
    b[..8].copy_from_slice(b"DOORCAT1");
    for (at, value, width) in [
        (8, 1, 4),
        (12, 16384, 4),
        (16, 20, 8),
        (24, 5, 8),
        (32, 2, 8),
        (44, 2, 4),
    ] {
        put(&mut b, at, value, width);
    }
    let names = [
        (1, 1, "root"),
        (2, 1, "system"),
        (3, 1, "users"),
        (4, 1, "state"),
        (5, 1, "apps"),
        (6, 1, "cache"),
        (7, 1, "tmp"),
        (8, 1, "volumes"),
        (9, 1, "devices"),
        (10, 1, "compat"),
        (11, 10, "Windows"),
        (12, 11, "System32"),
        (13, 2, "versions"),
        (14, 2, "current"),
        (15, 4, "registry"),
        (16, 4, "machine"),
        (17, 3, "1001"),
        (18, 17, if new { "revised-Ω.txt" } else { "note-Ω.txt" }),
        (19, 17, "keep.bin"),
    ];
    for (id, parent, name) in names {
        let r = &mut b[128 + (id - 1) * 224..128 + id * 224];
        put(r, 0, id as u64, 8);
        put(r, 8, parent, 8);
        r[16] = if id >= 18 { 1 } else { 2 };
        let units: Vec<_> = name.encode_utf16().collect();
        r[18] = units.len() as u8;
        for (i, u) in units.iter().enumerate() {
            put(r, 56 + i * 2, u64::from(*u), 2);
        }
        put(r, 24, 1, 8);
        let owner = if id <= 16 { 1 } else { 1001 };
        put(r, 152, owner, 4);
        r[157] = 1;
        r[160] = 1;
        put(r, 164, 63, 2);
        put(r, 168, owner, 4);
        if id >= 18 {
            put(r, 32, (id - 5) as u64, 4);
            put(r, 36, 1, 4);
            r[40..56].copy_from_slice(b"DoorP06Ext2UUID0");
        }
        if id == 18 {
            r[157] = 3;
            r[172] = 1;
            r[173] = 1;
            put(r, 176, if new { 3 } else { 2 }, 2);
            put(r, 180, 1002, 4);
            r[184] = 1;
            put(r, 188, 1, 2);
            put(r, 192, 1002, 4);
        }
    }
    for (i, name) in ["", "Software", "Door", "Fixture"].into_iter().enumerate() {
        let r = &mut b[7296 + i * 128..7424 + i * 128];
        put(r, 0, (i + 1) as u64, 8);
        put(r, 8, i as u64, 8);
        put(r, 16, 1001, 4);
        r[20] = 2;
        r[21] = 64;
        r[23] = name.len() as u8;
        put(r, 24, 1, 8);
        for (j, u) in name.encode_utf16().enumerate() {
            put(r, 32 + j * 2, u64::from(u), 2);
        }
    }
    let epoch = if new { 2u32 } else { 1u32 };
    for (i, (name, kind, data)) in [
        ("Epoch", 4, epoch.to_le_bytes().to_vec()),
        ("Bytes", 3, vec![0xd0, 7, epoch as u8, 0xa5]),
    ]
    .into_iter()
    .enumerate()
    {
        let r = &mut b[10368 + i * 192..10560 + i * 192];
        put(r, 0, 4, 8);
        r[8] = name.len() as u8;
        r[9] = kind;
        put(r, 10, data.len() as u64, 2);
        for (j, u) in name.encode_utf16().enumerate() {
            put(r, 16 + j * 2, u64::from(u), 2);
        }
        r[112..112 + data.len()].copy_from_slice(&data);
    }
    for (at, value, width) in [
        (14976, 1001, 4),
        (14980, 2, 4),
        (14984, if new { 2 } else { 1 }, 4),
        (14988, if new { 150 } else { 125 }, 4),
        (14992, 2, 8),
    ] {
        put(&mut b, at, value, width);
    }
    checksum(&mut b, 40);
    let file = |inode, bytes| tree::File {
        inode,
        generation: 1,
        bytes,
        blocks: vec![],
    };
    let mut files = BTreeMap::new();
    files.insert("seed.txt".into(), file(11, b"Door ext2 seed\n".to_vec()));
    files.insert("door.catalog".into(), file(12, b));
    files.insert(
        "obj0000000000000012".into(),
        file(
            13,
            (0..6000)
                .map(|i| {
                    if new {
                        ((i * 19 + 23) % 251) as u8
                    } else {
                        ((i * 37 + 11) % 251) as u8
                    }
                })
                .collect(),
        ),
    );
    files.insert(
        "obj0000000000000013".into(),
        file(14, b"Door P07 unrelated\n".to_vec()),
    );
    tree::Tree {
        files,
        free_blocks: 8175,
        free_inodes: 114,
    }
}
#[test]
fn catalog_audit_requires_atomic_names_data_acl_settings_and_registry_association() {
    for new in [false, true] {
        let mut t = catalog_tree(new);
        assert_eq!(
            catalog::inspect(&t).unwrap()["state"],
            if new { "new" } else { "old" }
        );
        for at in [
            128 + 17 * 224 + 32,
            128 + 17 * 224 + 152,
            14984,
            10368 + 112,
        ] {
            let b = &mut t.files.get_mut("door.catalog").unwrap().bytes;
            let old = b.clone();
            b[at] ^= 1;
            checksum(b, 40);
            assert!(catalog::inspect(&t).is_err());
            t.files.get_mut("door.catalog").unwrap().bytes = old;
        }
        t.files.get_mut("obj0000000000000012").unwrap().bytes[4096] ^= 1;
        assert!(catalog::inspect(&t).is_err());
    }
}
