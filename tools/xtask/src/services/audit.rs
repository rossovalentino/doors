//! Host-owned decoder: validates the shared disk format independently of guest
//! filesystem code, then checks the exact two-identity SDK workload policy.
use crate::persistence::{catalog, journal, tree};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

fn require(condition: bool, detail: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(detail.into())
    }
}
fn word(bytes: &[u8], at: usize, width: usize) -> u64 {
    bytes[at..at + width]
        .iter()
        .enumerate()
        .fold(0, |n, (i, b)| n | (u64::from(*b) << (8 * i)))
}
pub fn inspect(path: &Path) -> Result<Value, String> {
    inspect_mode(path, false)
}
pub(super) fn inspect_mode(path: &Path, corrupt: bool) -> Result<Value, String> {
    require(
        fs::metadata(path).map_err(|e| e.to_string())?.len() == crate::storage::DISK_BYTES as u64,
        "P08 private disk size",
    )?;
    inspect_kind(&fs::read(path).map_err(|e| e.to_string())?, corrupt)
}
pub(super) fn inspect_bytes(bytes: &[u8]) -> Result<Value, String> {
    inspect_kind(bytes, false)
}
pub(super) fn inspect_demo(path: &Path) -> Result<Value, String> {
    require(
        fs::metadata(path).map_err(|e| e.to_string())?.len() == crate::storage::DISK_BYTES as u64,
        "P08 private disk size",
    )?;
    inspect_profile(
        &fs::read(path).map_err(|e| e.to_string())?,
        false,
        Profile::Demo,
    )
}
pub(super) fn inspect_rename(path: &Path) -> Result<Value, String> {
    require(
        fs::metadata(path).map_err(|e| e.to_string())?.len() == crate::storage::DISK_BYTES as u64,
        "SDK rename private disk size",
    )?;
    inspect_profile(
        &fs::read(path).map_err(|e| e.to_string())?,
        false,
        Profile::RenameConsole,
    )
}
pub fn inspect_windows(path: &Path) -> Result<Value, String> {
    require(
        fs::metadata(path).map_err(|e| e.to_string())?.len() == crate::storage::DISK_BYTES as u64,
        "P10 private disk size",
    )?;
    inspect_profile(
        &fs::read(path).map_err(|e| e.to_string())?,
        false,
        Profile::Windows,
    )
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum Profile {
    Native,
    Demo,
    Windows,
    RenameConsole,
}
fn inspect_kind(bytes: &[u8], corrupt: bool) -> Result<Value, String> {
    inspect_profile(bytes, corrupt, Profile::Native)
}
fn inspect_profile(bytes: &[u8], corrupt: bool, profile: Profile) -> Result<Value, String> {
    let demo = profile == Profile::Demo;
    let windows = profile == Profile::Windows;
    crate::persistence::gpt(bytes)?;
    let fat = crate::storage::audit::fat(bytes)?;
    let journal = journal::inspect(bytes)?;
    let home = journal.replayed_home(bytes)?;
    let tree = tree::inspect(&home)?;
    let catalog = if corrupt {
        require(
            matches!(catalog::decode(&tree), Err(ref reason) if reason == "catalog CRC"),
            "P08 expected corrupt catalog CRC refusal",
        )?;
        json!({"state":"corrupt","reason":"catalog CRC"})
    } else {
        inspect_catalog(&tree, if demo { 2 } else { 1 }, windows)?
    };
    let generation = if catalog["state"] == "initial" {
        0
    } else if profile == Profile::RenameConsole {
        5
    } else if demo || windows {
        4
    } else {
        3
    };
    require(
        !journal.selected().committed && journal.selected().generation == generation,
        "P08 final journal checkpoint",
    )?;
    Ok(
        json!({"schema_version":1,"record_kind":"services_audit","bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes)),"gpt":{"partitions":3,"primary":true,"backup":true},"fat32":fat,"journal":journal.summary(),"effective_home_sha256":format!("{:x}",Sha256::digest(&home)),"ext2":tree.summary(),"catalog":catalog}),
    )
}

fn inspect_catalog(tree: &tree::Tree, theme: u64, windows: bool) -> Result<Value, String> {
    let Some(catalog) = catalog::decode(tree)? else {
        return Ok(json!({"state":"initial"}));
    };
    let paths = catalog.paths()?;
    let bytes = catalog.bytes;
    require(
        paths.len() == if windows { 21 } else { 20 }
            && catalog.associations.len() == if windows { 3 } else { 2 }
            && catalog.next_object == if windows { 22 } else { 21 },
        "P08 expected eighteen directories and two files",
    )?;
    for node in paths.values().filter(|node| node.id <= 16) {
        require(
            node.kind == 2
                && node.owner == 1
                && node.integrity == 1
                && node.attributes == 0
                && node.aces == [(1, 1, false, 63), (0, 0, false, 9)]
                && node.inherited == [true, true],
            "P08 protected directory ACL changed",
        )?;
    }
    inspect_registry(bytes, catalog.next_key)?;
    let mut file_records = Vec::new();
    for (user, seed) in [(1001, 17), (1002, 53)] {
        let directory_path = format!("/users/{user}");
        let file_path = format!("{directory_path}/workload.bin");
        let directory = paths
            .get(&directory_path)
            .ok_or("P08 user directory missing")?;
        let file = paths.get(&file_path).ok_or("P08 workload file missing")?;
        require(
            directory.kind == 2 && file.kind == 1 && file.parent == directory.id,
            "P08 workload hierarchy",
        )?;
        for node in [directory, file] {
            require(
                node.owner == user
                    && node.integrity == 0
                    && node.attributes == 0
                    && node.aces == [(1, user, false, 63)]
                    && node.inherited == [true]
                    && catalog::allows(node, user, 63)
                    && !catalog::allows(node, if user == 1001 { 1002 } else { 1001 }, 1),
                "P08 workload owner/inherited ACL",
            )?;
        }
        let backing = &tree.files[&format!("obj{:016x}", file.id)];
        let expected: Vec<_> = (0..4096).map(|i| ((i * 29 + seed) % 251) as u8).collect();
        require(
            backing.bytes == expected,
            "P08 complete concurrent file contents",
        )?;
        file_records.push(json!({"path":file_path,"owner":user,"object":file.id,"inode":file.inode,"generation":file.generation,"bytes":backing.bytes.len(),"sha256":format!("{:x}",Sha256::digest(&backing.bytes))}));
    }
    let mut settings = BTreeMap::new();
    if windows {
        let path = "/users/1001/p10-data-λ.bin";
        let file = paths.get(path).ok_or("P10 persisted file missing")?;
        require(
            file.id == 21
                && file.kind == 1
                && file.parent == paths["/users/1001"].id
                && file.owner == 1001
                && file.integrity == 0
                && file.attributes == 0
                && file.aces == [(1, 1001, false, 63)]
                && file.inherited == [true]
                && catalog::allows(file, 1001, 63)
                && !catalog::allows(file, 1002, 1),
            "P10 file identity and ACL",
        )?;
        let backing = &tree.files[&format!("obj{:016x}", file.id)];
        let expected: Vec<_> = (0..1024)
            .map(|index| (index % 512 + 1 + index / 512) as u8)
            .collect();
        require(
            backing.bytes == expected,
            "P10 exact independently computed data-processing bytes",
        )?;
        file_records.push(json!({"path":path,"owner":1001,"object":file.id,"inode":file.inode,"generation":file.generation,"bytes":backing.bytes.len(),"sha256":format!("{:x}",Sha256::digest(&backing.bytes))}));
    }
    for record in bytes[14976..15488].chunks_exact(64) {
        let schema = word(record, 4, 4);
        if schema == 0 {
            require(record.iter().all(|b| *b == 0), "P08 empty setting record")?;
            continue;
        }
        let owner = word(record, 0, 4);
        require(
            matches!(schema, 2..=4)
                && word(record, 16, 8) > 0
                && record[24..].iter().all(|b| *b == 0),
            "P08 native setting encoding",
        )?;
        require(
            settings
                .insert(owner, (word(record, 8, 4), word(record, 12, 4)))
                .is_none(),
            "P08 duplicate settings owner",
        )?;
    }
    require(
        settings == BTreeMap::from([(1001, (theme, 125)), (1002, (2, 150))]),
        "P08 per-identity theme/scale",
    )?;
    Ok(
        json!({"state":"saved","revision":catalog.revision,"next_object":catalog.next_object,"next_key":catalog.next_key,"paths":paths.keys().collect::<Vec<_>>(),"files":file_records,"settings":settings,"registry_keys":4,"registry_values":2}),
    )
}

fn inspect_registry(bytes: &[u8], next_key: u64) -> Result<(), String> {
    require(next_key == 5, "P08 registry next key")?;
    let mut keys = BTreeMap::new();
    for record in bytes[7296..10368].chunks_exact(128) {
        let id = word(record, 0, 8);
        if id == 0 {
            require(record.iter().all(|b| *b == 0), "P08 empty registry key")?;
            continue;
        }
        let owner = word(record, 16, 4);
        require(
            id < next_key
                && matches!(owner, 1001 | 1002)
                && record[20] == 2
                && record[21] == 64
                && record[22] == 0
                && word(record, 24, 8) > 0,
            "P08 registry key authority/view",
        )?;
        let name = catalog::name(record, 32, record[23] as usize)?;
        require(
            keys.insert(id, (word(record, 8, 8), owner, name)).is_none(),
            "P08 duplicate registry key",
        )?;
    }
    require(keys.len() == 4, "P08 registry key count")?;
    let mut owners = BTreeMap::new();
    for owner in [1001, 1002] {
        let root: Vec<_> = keys
            .iter()
            .filter(|(_, value)| value.0 == 0 && value.1 == owner && value.2.is_empty())
            .collect();
        require(root.len() == 1, "P08 per-user registry root")?;
        let door: Vec<_> = keys
            .iter()
            .filter(|(_, value)| value.0 == *root[0].0 && value.1 == owner && value.2 == "Door")
            .collect();
        require(door.len() == 1, "P08 per-user Door registry key")?;
        owners.insert(*door[0].0, owner);
    }
    let mut seen = std::collections::BTreeSet::new();
    for record in bytes[10368..14976].chunks_exact(192) {
        let key = word(record, 0, 8);
        if key == 0 {
            require(record.iter().all(|b| *b == 0), "P08 empty registry value")?;
            continue;
        }
        let owner = *owners
            .get(&key)
            .ok_or("P08 registry value on wrong owner/key")?;
        require(
            catalog::name(record, 16, record[8] as usize)? == "Epoch"
                && record[9] == 4
                && word(record, 10, 2) == 4
                && record[12..16].iter().all(|b| *b == 0)
                && word(record, 112, 4) == owner
                && record[116..].iter().all(|b| *b == 0)
                && seen.insert(key),
            "P08 typed per-user registry value",
        )?;
    }
    require(seen.len() == 2, "P08 registry value count")
}

pub(super) fn corruption_offset(bytes: &[u8]) -> Result<usize, String> {
    let checked = inspect_bytes(bytes)?;
    require(
        checked["catalog"]["state"] == "saved",
        "P08 corruption requires saved baseline",
    )?;
    let journal = journal::inspect(bytes)?;
    let home = journal.replayed_home(bytes)?;
    let tree = tree::inspect(&home)?;
    let catalog = tree
        .files
        .get("door.catalog")
        .ok_or("P08 baseline catalog")?;
    Ok(133120 * 512 + catalog.blocks.first().ok_or("P08 catalog blocks")? * 4096 + 40)
}

pub(super) fn validate_demo_change(before: &Path, after: &Path) -> Result<(), String> {
    inspect(before)?;
    inspect_demo(after)?;
    let old = fs::read(before).map_err(|e| e.to_string())?;
    let new = fs::read(after).map_err(|e| e.to_string())?;
    let journal = journal::inspect(&old)?;
    let home = journal.replayed_home(&old)?;
    let tree = tree::inspect(&home)?;
    let catalog = tree
        .files
        .get("door.catalog")
        .ok_or("console initial catalog")?;
    let settings = catalog.bytes[14976..15488]
        .chunks_exact(64)
        .position(|r| word(r, 0, 4) == 1001)
        .ok_or("console initial owner settings")?;
    let setting = 14976 + settings * 64;
    let physical = |at: usize| 133120 * 512 + catalog.blocks[at / 4096] * 4096 + at % 4096;
    let mut allowed = std::collections::BTreeSet::new();
    for range in [
        32..44,
        setting + 8..setting + 12,
        setting + 16..setting + 24,
    ] {
        allowed.extend(range.map(physical));
    }
    require(
        old.iter().zip(&new).enumerate().all(|(at, (a, b))| {
            a == b || allowed.contains(&at) || (202752 * 512..206848 * 512).contains(&at)
        }),
        "console theme command changed unrelated persisted bytes",
    )?;
    for at in [32, setting + 16] {
        let old_value = word(&old, physical(at), 8);
        let new_value = word(&new, physical(at), 8);
        require(
            old_value.checked_add(1) == Some(new_value),
            "console theme change did not advance exact setting/catalog revisions",
        )?;
    }
    Ok(())
}

/// Called after both complete independent disk audits. Only the catalog CRC,
/// catalog revision and this one source version may differ after a roundtrip,
/// plus (catalog v4/v5) the parent folder's modified time. Returns the source
/// version offset and that folder time range.
fn rename_catalog_change(
    old: &[u8],
    new: &[u8],
    source: u64,
) -> Result<(usize, core::ops::Range<usize>), String> {
    require(
        old.len() == new.len() && old.len() >= 7296,
        "SDK rename catalog bounds",
    )?;
    let index = old[128..7296]
        .chunks_exact(224)
        .position(|record| word(record, 0, 8) == source)
        .ok_or("SDK rename source record absent")?;
    let version = 128 + index * 224 + 24;
    // The parent folder modified time (catalog v4/v5) follows the rename.
    let parent = word(old, 128 + index * 224 + 8, 8);
    let folder = crate::persistence::catalog::record_offset(old, parent)
        .map_or(0..0, |(start, _)| start + 216..start + 224);
    for at in [32, version] {
        require(
            word(old, at, 8).checked_add(2) == Some(word(new, at, 8)),
            "SDK rename must advance exactly two catalog/source versions",
        )?;
    }
    require(
        old.iter().zip(new).enumerate().all(|(at, (a, b))| {
            a == b
                || (32..44).contains(&at)
                || (version..version + 8).contains(&at)
                || folder.contains(&at)
        }),
        "SDK rename changed unrelated catalog state",
    )?;
    Ok((version, folder))
}
pub(super) fn validate_rename_change(before: &Path, after: &Path) -> Result<(), String> {
    inspect(before)?;
    inspect_rename(after)?;
    let old = fs::read(before).map_err(|e| e.to_string())?;
    let new = fs::read(after).map_err(|e| e.to_string())?;
    let journal = journal::inspect(&old)?;
    let home = journal.replayed_home(&old)?;
    let tree = tree::inspect(&home)?;
    let catalog = catalog::decode(&tree)?.ok_or("SDK rename baseline catalog")?;
    let paths = catalog.paths()?;
    let source = paths
        .get("/users/1001/workload.bin")
        .ok_or("SDK rename baseline source")?;
    let next_journal = journal::inspect(&new)?;
    let next_home = next_journal.replayed_home(&new)?;
    let next_tree = tree::inspect(&next_home)?;
    let next_catalog = catalog::decode(&next_tree)?.ok_or("SDK rename final catalog")?;
    let (version, folder) = rename_catalog_change(catalog.bytes, next_catalog.bytes, source.id)?;
    let backing = tree
        .files
        .get("door.catalog")
        .ok_or("SDK rename catalog backing")?;
    let physical = |at: usize| 133120 * 512 + backing.blocks[at / 4096] * 4096 + at % 4096;
    let allowed: std::collections::BTreeSet<_> = (32..44)
        .chain(version..version + 8)
        .chain(folder)
        .map(physical)
        .collect();
    require(
        old.iter().zip(&new).enumerate().all(|(at, (a, b))| {
            a == b || allowed.contains(&at) || (202752 * 512..206848 * 512).contains(&at)
        }),
        "SDK rename changed unrelated disk bytes, allocation, metadata or file content",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_roundtrip_changes_only_two_versions_and_crc() {
        let tree = fixture();
        let old = tree.files["door.catalog"].bytes.clone();
        let mut new = old.clone();
        let version = 128 + 18 * 224 + 24;
        for at in [32, version] {
            put(&mut new, at, word(&old, at, 8) + 2, 8);
        }
        checksum(&mut new);
        assert_eq!(rename_catalog_change(&old, &new, 19).unwrap().0, version);
        // The parent folder (17) modified time may follow; nothing else.
        let folder = 128 + 16 * 224 + 216;
        assert_eq!(
            rename_catalog_change(&old, &new, 19).unwrap().1,
            folder..folder + 8
        );
        let mut stamped = new.clone();
        put(&mut stamped, folder, 133_000_000_000_000_000, 8);
        checksum(&mut stamped);
        assert!(rename_catalog_change(&old, &stamped, 19).is_ok());
        let mut other = new.clone();
        put(&mut other, 128 + 17 * 224 + 216, 1, 8);
        checksum(&mut other);
        assert!(rename_catalog_change(&old, &other, 19).is_err());
        for at in [16, 24, version, version + 8, 128 + 18 * 224 + 56, 14984] {
            let mut bad = new.clone();
            bad[at] ^= 1;
            checksum(&mut bad);
            assert!(rename_catalog_change(&old, &bad, 19).is_err(), "{at}");
        }
        assert!(rename_catalog_change(&old, &old, 19).is_err());
        assert!(rename_catalog_change(&old, &new, 999).is_err());
        assert!(rename_catalog_change(&old, &new[..7295], 19).is_err());
    }
    fn put(bytes: &mut [u8], at: usize, value: u64, width: usize) {
        bytes[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
    }
    fn checksum(bytes: &mut [u8]) {
        bytes[40..44].fill(0);
        let crc = crate::storage::audit::crc32(bytes);
        put(bytes, 40, u64::from(crc), 4);
    }
    fn fixture() -> tree::Tree {
        let mut tree = crate::persistence::tests::catalog_tree(false);
        tree.files
            .retain(|name, _| name == "door.catalog" || name == "seed.txt");
        let bytes = &mut tree.files.get_mut("door.catalog").unwrap().bytes;
        put(bytes, 16, 21, 8);
        put(bytes, 24, 5, 8);
        bytes[128 + 16 * 224..].fill(0);
        for record in bytes[128..128 + 16 * 224].chunks_exact_mut(224) {
            record[156] = 1;
            record[157] = 2;
            record[162] = 1;
            record[174] = 1;
            put(record, 176, 9, 2);
        }
        for (id, parent, kind, name, owner, inode) in [
            (17, 3, 2, "1001", 1001, 0),
            (18, 3, 2, "1002", 1002, 0),
            (19, 17, 1, "workload.bin", 1001, 13),
            (20, 18, 1, "workload.bin", 1002, 14),
        ] {
            let record = &mut bytes[128 + (id - 1) * 224..128 + id * 224];
            put(record, 0, id as u64, 8);
            put(record, 8, parent, 8);
            record[16] = kind;
            record[18] = name.len() as u8;
            put(record, 24, 1, 8);
            for (index, unit) in name.encode_utf16().enumerate() {
                put(record, 56 + index * 2, u64::from(unit), 2);
            }
            if kind == 1 {
                put(record, 32, inode, 4);
                put(record, 36, 1, 4);
                record[40..56].copy_from_slice(b"DoorP06Ext2UUID0");
            }
            put(record, 152, owner, 4);
            record[157] = 1;
            record[160] = 1;
            record[162] = 1;
            put(record, 164, 63, 2);
            put(record, 168, owner, 4);
        }
        for (index, owner, theme, scale) in [(0, 1001, 1, 125), (1, 1002, 2, 150)] {
            let record = &mut bytes[14976 + index * 64..15040 + index * 64];
            put(record, 0, owner, 4);
            put(record, 4, 2, 4);
            put(record, 8, theme, 4);
            put(record, 12, scale, 4);
            put(record, 16, 2, 8);
        }
        for (index, owner, parent, name) in [
            (0, 1001, 0, ""),
            (1, 1001, 1, "Door"),
            (2, 1002, 0, ""),
            (3, 1002, 3, "Door"),
        ] {
            let record = &mut bytes[7296 + index * 128..7424 + index * 128];
            put(record, 0, (index + 1) as u64, 8);
            put(record, 8, parent, 8);
            put(record, 16, owner, 4);
            record[20] = 2;
            record[21] = 64;
            record[23] = name.len() as u8;
            put(record, 24, 1, 8);
            for (index, unit) in name.encode_utf16().enumerate() {
                put(record, 32 + index * 2, u64::from(unit), 2);
            }
        }
        for (index, key, owner) in [(0, 2, 1001), (1, 4, 1002)] {
            let record = &mut bytes[10368 + index * 192..10560 + index * 192];
            put(record, 0, key, 8);
            record[8] = 5;
            record[9] = 4;
            put(record, 10, 4, 2);
            for (index, unit) in "Epoch".encode_utf16().enumerate() {
                put(record, 16 + index * 2, u64::from(unit), 2);
            }
            put(record, 112, owner, 4);
        }
        checksum(bytes);
        for (object, inode, seed) in [(19, 13, 17), (20, 14, 53)] {
            tree.files.insert(
                format!("obj{object:016x}"),
                tree::File {
                    inode,
                    generation: 1,
                    bytes: (0..4096).map(|i| ((i * 29 + seed) % 251) as u8).collect(),
                    blocks: vec![],
                },
            );
        }
        tree
    }
    #[test]
    fn exact_sdk_bytes_owners_settings_and_namespace_are_required() {
        let mut tree = fixture();
        assert_eq!(inspect_catalog(&tree, 1, false).unwrap()["state"], "saved");
        let original = tree.files["door.catalog"].bytes.clone();
        for index in 0..2 {
            let bytes = &mut tree.files.get_mut("door.catalog").unwrap().bytes;
            put(bytes, 14980 + index * 64, 3, 4);
            checksum(bytes);
        }
        assert_eq!(inspect_catalog(&tree, 1, false).unwrap()["state"], "saved");
        {
            let bytes = &mut tree.files.get_mut("door.catalog").unwrap().bytes;
            put(bytes, 15000, 1, 4);
            checksum(bytes);
        }
        assert!(inspect_catalog(&tree, 1, false).is_err());
        tree.files
            .get_mut("door.catalog")
            .unwrap()
            .bytes
            .clone_from(&original);
        for at in [
            16,
            24,
            128 + 164,
            128 + 16 * 224 + 152,
            128 + 16 * 224 + 162,
            128 + 18 * 224 + 20,
            128 + 18 * 224 + 36,
            7296,
            7296 + 128 + 16,
            7296 + 128 + 21,
            10368 + 112,
            10368 + 192 + 112,
            14984,
            15052,
        ] {
            let bytes = &mut tree.files.get_mut("door.catalog").unwrap().bytes;
            bytes[at] ^= 1;
            checksum(bytes);
            assert!(
                inspect_catalog(&tree, 1, false).is_err(),
                "accepted mutation at {at}"
            );
            tree.files
                .get_mut("door.catalog")
                .unwrap()
                .bytes
                .clone_from(&original);
        }
        for at in [0, 2047, 2048, 4095] {
            tree.files.get_mut("obj0000000000000013").unwrap().bytes[at] ^= 1;
            assert!(inspect_catalog(&tree, 1, false).is_err());
            tree.files.get_mut("obj0000000000000013").unwrap().bytes[at] ^= 1;
        }
        tree.files.remove("obj0000000000000014");
        assert!(inspect_catalog(&tree, 1, false).is_err());
    }
    #[test]
    fn original_disk_is_initial_and_cannot_be_corrupted_as_a_saved_fixture() {
        let bytes = crate::persistence::format::image(false);
        assert_eq!(
            inspect_bytes(&bytes).unwrap()["catalog"]["state"],
            "initial"
        );
        assert!(inspect_kind(&bytes, true).is_err());
        assert!(corruption_offset(&bytes).is_err());
    }
}
