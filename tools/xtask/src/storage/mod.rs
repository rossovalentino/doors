//! Reusable P06 raw-disk fixture generation and independent persistence evidence.
pub mod audit;
pub(crate) mod format;

use crate::{
    evidence::{Run, relative, write_json},
    paths,
};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub const PROFILE: &str = "profiles/hardware/qemu-storage-x64.json";
pub const DISK_BYTES: usize = 128 * 1024 * 1024;
const FAT_START: usize = 2048;
const EXT2_START: usize = 133120;
pub fn validate_sources(report: &Value) -> Result<(), String> {
    let sources = report["source_snapshot"]["files"]
        .as_array()
        .ok_or("storage source identity missing")?;
    for required in [
        "tools/xtask/src/storage/format.rs",
        "tools/xtask/src/storage/audit.rs",
        "tools/xtask/src/storage/mod.rs",
        "profiles/hardware/qemu-storage-x64.json",
        "kernel/src/storage_diagnostics.rs",
        "kernel/src/drivers/pci.rs",
        "kernel/src/drivers/queue.rs",
        "kernel/src/drivers/virtio_pci.rs",
        "kernel/src/drivers/virtio_block.rs",
        "kernel/src/storage/block.rs",
        "kernel/src/storage/gpt.rs",
        "kernel/src/storage/fat32.rs",
        "kernel/src/storage/ext2.rs",
        "kernel/src/storage/cache.rs",
        "kernel/src/storage/vfs.rs",
        "specs/contracts/storage-filesystems-v1.md",
        "specs/contracts/storage-transport-v1.md",
    ] {
        let required = crate::phase::storage_source(report, required);
        let matching: Vec<_> = sources
            .iter()
            .filter(|file| file["path"] == required)
            .collect();
        if matching.len() != 1
            || matching[0]["sha256"]
                .as_str()
                .is_none_or(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(format!(
                "storage source identity missing/duplicated: {required}"
            ));
        }
    }
    Ok(())
}

pub fn create(run: &Run) -> Result<String, String> {
    let directory = paths::fresh_directory(&run.root, &format!("out/storage/{}", run.id))?;
    let bytes = format::image();
    let checked = audit::inspect(&bytes, false)?;
    fs::write(directory.join("initial.raw"), &bytes).map_err(|e| e.to_string())?;
    fs::write(directory.join("data.raw"), &bytes).map_err(|e| e.to_string())?;
    write_json(&directory.join("initial-audit.json"), &checked)?;
    let name = relative(&run.root, &directory.join("data.raw"))?;
    println!(
        "Storage fixture: {name}; expected persistence CRC32 {}",
        audit::payload_crc()
    );
    Ok(name)
}
pub fn inspect(path: &Path, persisted: bool) -> Result<Value, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() != DISK_BYTES as u64 {
        return Err("storage fixture length mismatch".into());
    }
    audit::inspect(&fs::read(path).map_err(|e| e.to_string())?, persisted)
}
pub fn confined(root: &Path, name: &str) -> Result<std::path::PathBuf, String> {
    let relative = paths::relative_path(name)?;
    if !relative.starts_with("out/storage")
        || relative.components().count() != 4
        || relative.file_name().and_then(|p| p.to_str()) != Some("data.raw")
    {
        return Err("writable storage must be an out/storage/<session>/data.raw fixture".into());
    }
    let path = paths::existing(root, name)?;
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() != DISK_BYTES as u64 {
        return Err("writable storage size mismatch".into());
    }
    Ok(path)
}
pub fn serial_matches(markers: &[&str], serial: &str) -> bool {
    for marker in markers.iter().filter(|m| m.starts_with("DOOR_P06 ")) {
        let prefix = marker.split(" PASS").next().unwrap_or(marker);
        let lines: Vec<_> = serial
            .lines()
            .filter(|l| {
                *l == prefix
                    || l.strip_prefix(prefix)
                        .is_some_and(|tail| tail.starts_with(' '))
            })
            .collect();
        if lines.len() != 1 {
            return false;
        }
        let line = lines[0];
        if *marker == "DOOR_P06 BLOCK PASS" {
            let Some(tail) = line.strip_prefix("DOOR_P06 BLOCK PASS ") else {
                return false;
            };
            let fields: Vec<_> = tail.split(' ').collect();
            if fields.len() != 3 {
                return false;
            }
            let values = fields
                .iter()
                .zip(["reads=", "writes=", "flushes="])
                .map(|(text, key)| {
                    let value = text.strip_prefix(key)?;
                    let n = value.parse::<u64>().ok()?;
                    (n.to_string() == value).then_some(n)
                })
                .collect::<Option<Vec<_>>>();
            let Some(v) = values else {
                return false;
            };
            if v[0] == 0
                || v[2] == 0
                || (markers.contains(&"DOOR_P06 PERSIST PASS stage=write") && v[1] == 0)
            {
                return false;
            }
        } else if *marker == "DOOR_P06 STACK PASS" {
            let Some(tail) = line.strip_prefix("DOOR_P06 STACK PASS sampled_remaining=") else {
                return false;
            };
            let Some(text) = tail.strip_suffix(" stack_bytes=131072") else {
                return false;
            };
            let Ok(remaining) = text.parse::<u64>() else {
                return false;
            };
            if !(16384..=131072).contains(&remaining) || remaining.to_string() != text {
                return false;
            }
        } else if *marker == "DOOR_P06 DEVICE PASS" {
            let Some(tail) = line.strip_prefix("DOOR_P06 DEVICE PASS pci=") else {
                return false;
            };
            let Some((bdf, rest)) = tail.split_once(' ') else {
                return false;
            };
            if !bdf.is_ascii()
                || bdf.len() != 7
                || bdf.as_bytes()[2] != b':'
                || bdf.as_bytes()[5] != b'.'
                || u8::from_str_radix(&bdf[..2], 16).is_err()
                || !u8::from_str_radix(&bdf[3..5], 16).is_ok_and(|n| n <= 31)
                || !u8::from_str_radix(&bdf[6..], 16).is_ok_and(|n| n <= 7)
                || rest != "features=0000000100000200 queue=16 dma_pages=8"
            {
                return false;
            }
        } else if marker.starts_with("DOOR_P06 PERSIST ") {
            if line != format!("{marker} bytes=6000 crc32={}", audit::payload_crc()) {
                return false;
            }
        } else if line != *marker {
            return false;
        }
    }
    true
}
pub fn suite_evidence(root: &Path, name: &str) -> Result<Value, String> {
    let path = confined(root, name)?;
    let initial = path
        .parent()
        .ok_or("storage directory")?
        .join("initial.raw");
    paths::reject_links(root, &initial)?;
    let initial_audit = inspect(&initial, false)?;
    let final_audit = inspect(&path, true)?;
    Ok(
        json!({"schema_version":1,"record_kind":"storage_session","path":name,"initial":{"path":relative(root,&initial)?,"sha256":initial_audit["sha256"],"audit":initial_audit},"final":{"path":name,"sha256":final_audit["sha256"],"audit":final_audit},"expected_payload_crc32":audit::payload_crc()}),
    )
}
pub fn validate_suite(root: &Path, suite: &Value, guests: &[Value]) -> Result<(), String> {
    if guests.len() != 2
        || guests[0]["scenario"] != "storage-write"
        || guests[1]["scenario"] != "storage-reopen"
    {
        return Err("storage requires ordered write and reopen guests".into());
    }
    let declared = &suite["storage"];
    let name = declared["path"]
        .as_str()
        .ok_or("storage session path missing")?;
    if suite_evidence(root, name)? != *declared {
        return Err("storage session independent audit differs".into());
    }
    let expected_initial = audit::inspect(&format::image(), false)?;
    if declared["initial"]["audit"] != expected_initial {
        return Err("storage initial image differs from deterministic formatter".into());
    }
    let mut previous = declared["initial"]["audit"].clone();
    for (index, guest) in guests.iter().enumerate() {
        let mode = if index == 0 {
            "storage-write"
        } else {
            "storage-reopen"
        };
        let data = &guest["data_disk"];
        if guest["scenario"] != mode
            || data["path"] != name
            || data["before"] != previous
            || !data["audit_error"].is_null()
        {
            return Err("storage boot hash chain differs".into());
        }
        let expected_path = format!(
            "out/qemu/{}/data-after.raw",
            guest["run_token"].as_str().ok_or("storage run token")?
        );
        if data["after"]["path"] != expected_path {
            return Err("storage snapshot path not bound to guest".into());
        }
        let path = paths::existing(root, &expected_path)?;
        let actual = inspect(&path, true)?;
        if data["after"]["audit"] != actual || data["after"]["sha256"] != actual["sha256"] {
            return Err("storage snapshot bytes differ".into());
        }
        if index == 1 {
            let prior = paths::existing(
                root,
                guests[0]["data_disk"]["after"]["path"]
                    .as_str()
                    .ok_or("write snapshot")?,
            )?;
            immutable_except_scratch(
                &fs::read(prior).map_err(|e| e.to_string())?,
                &fs::read(&path).map_err(|e| e.to_string())?,
            )?;
        }
        previous = actual;
    }
    if previous != declared["final"]["audit"] || previous["sha256"] == declared["initial"]["sha256"]
    {
        return Err("storage final bytes do not prove persisted mutation".into());
    }
    // The guest may modify ext2 and one explicit reset-probe sector. FAT and
    // both GPT copies remain exact; the scratch extent is outside every partition.
    let initial = paths::existing(
        root,
        declared["initial"]["path"].as_str().ok_or("initial path")?,
    )?;
    let final_path = confined(root, name)?;
    let a = fs::read(initial).map_err(|e| e.to_string())?;
    let b = fs::read(final_path).map_err(|e| e.to_string())?;
    if a[..EXT2_START * 512] != b[..EXT2_START * 512]
        || a[198656 * 512..200000 * 512] != b[198656 * 512..200000 * 512]
        || a[200001 * 512..] != b[200001 * 512..]
    {
        return Err(
            "guest modified bytes outside native partition and one-sector reset probe".into(),
        );
    }
    Ok(())
}
fn immutable_except_scratch(a: &[u8], b: &[u8]) -> Result<(), String> {
    if a.len() != DISK_BYTES
        || b.len() != DISK_BYTES
        || a[..200000 * 512] != b[..200000 * 512]
        || a[200001 * 512..] != b[200001 * 512..]
    {
        return Err("storage reopen changed bytes outside one-sector reset probe".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_formatter_passes_independent_layout_and_allocation_audit() {
        let image = format::image();
        assert_eq!(image, format::image());
        let record = audit::inspect(&image, false).unwrap();
        assert_eq!(record["ext2"]["free_blocks"], 8182);
        assert_eq!(record["ext2"]["free_inodes"], 117);
        assert!(audit::inspect(&image, true).is_err());
        assert_eq!(audit::crc32(b"123456789"), 0xcbf4_3926);
    }
    #[test]
    fn auditor_rejects_corrupt_gpt_fat_metadata_and_allocation_leaks() {
        let mut image = format::image();
        for at in [
            512 + 16,
            262143 * 512 + 88,
            2048 * 512 + 488 + 512,
            2048 * 512 + 32 * 512 + 12,
            2048 * 512 + 2051 * 512,
            133120 * 512 + 1024 + 12,
            133120 * 512 + 8192 + 1,
            133120 * 512 + 12288 + 1,
            133120 * 512 + 32768 + 4,
            133120 * 512 + 9 * 4096,
        ] {
            image[at] ^= 1;
            assert!(
                audit::inspect(&image, false).is_err(),
                "accepted corruption at {at}"
            );
            image[at] ^= 1;
        }
        assert!(audit::inspect(&image[..image.len() - 1], false).is_err());
    }
    fn persisted_fixture() -> Vec<u8> {
        let mut image = format::image();
        let at = 133120 * 512;
        let fs = &mut image[at..198656 * 512];
        fs[1024 + 12..1024 + 16].copy_from_slice(&8180u32.to_le_bytes());
        fs[1024 + 16..1024 + 20].copy_from_slice(&116u32.to_le_bytes());
        fs[4096 + 12..4096 + 14].copy_from_slice(&8180u16.to_le_bytes());
        fs[4096 + 14..4096 + 16].copy_from_slice(&116u16.to_le_bytes());
        fs[8193] = 15;
        fs[12289] = 15;
        let inode = &mut fs[16384 + 11 * 128..16384 + 12 * 128];
        inode[..2].copy_from_slice(&0x81a4u16.to_le_bytes());
        inode[4..8].copy_from_slice(&6000u32.to_le_bytes());
        inode[26..28].copy_from_slice(&1u16.to_le_bytes());
        inode[28..32].copy_from_slice(&16u32.to_le_bytes());
        inode[40..44].copy_from_slice(&10u32.to_le_bytes());
        inode[44..48].copy_from_slice(&11u32.to_le_bytes());
        fs[32768 + 28..32768 + 30].copy_from_slice(&16u16.to_le_bytes());
        let entry = &mut fs[32768 + 40..36864];
        entry[..4].copy_from_slice(&12u32.to_le_bytes());
        entry[4..6].copy_from_slice(&4056u16.to_le_bytes());
        entry[6] = 11;
        entry[7] = 1;
        entry[8..19].copy_from_slice(b"persist.bin");
        fs[40960..40960 + 6000].copy_from_slice(&audit::payload());
        image
    }
    #[test]
    fn persisted_audit_requires_final_name_contents_and_exact_reachable_allocations() {
        let mut image = persisted_fixture();
        let checked = audit::inspect(&image, true).unwrap();
        assert_eq!(checked["ext2"]["persistence"]["crc32"], "aaa29e86");
        assert_eq!(checked["ext2"]["free_blocks"], 8180);
        assert_eq!(checked["ext2"]["free_inodes"], 116);
        assert!(audit::inspect(&image, false).is_err());
        for at in [
            133120 * 512 + 40960 + 3584,
            133120 * 512 + 40960 + 4096,
            133120 * 512 + 32768 + 48,
            133120 * 512 + 16384 + 11 * 128 + 44,
        ] {
            image[at] ^= 1;
            assert!(audit::inspect(&image, true).is_err());
            image[at] ^= 1;
        }
    }
    #[test]
    fn p06_preserves_old_cases_and_rejects_weakened_numeric_device_and_persistence_markers() {
        let old = crate::scenarios::cases("P05").unwrap();
        let all = crate::scenarios::cases("P06").unwrap();
        assert_eq!(all.len(), 17);
        for (a, b) in old.iter().zip(&all) {
            assert_eq!(
                (a.id, a.mode, a.profile, &a.markers),
                (b.id, b.mode, b.profile, &b.markers)
            );
        }
        let markers = &all[15].markers;
        let serial = markers
            .iter()
            .filter(|m| m.starts_with("DOOR_P06 "))
            .map(|m| match *m {
                "DOOR_P06 BLOCK PASS" => format!("{m} reads=41 writes=15 flushes=8"),
                "DOOR_P06 STACK PASS" => format!("{m} sampled_remaining=24576 stack_bytes=131072"),
                "DOOR_P06 DEVICE PASS" => {
                    format!("{m} pci=00:03.0 features=0000000100000200 queue=16 dma_pages=8")
                }
                "DOOR_P06 PERSIST PASS stage=write" => format!("{m} bytes=6000 crc32=aaa29e86"),
                _ => (*m).to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(serial_matches(markers, &serial));
        for (from, to) in [
            ("reads=41", "reads=0"),
            ("writes=15", "writes=0"),
            ("flushes=8", "flushes=0"),
            ("pci=00:03.0", "pci=00:ff.0"),
            ("pci=00:03.0", "pci=00:03.8"),
            ("features=0000000100000200", "features=0000000100000000"),
            ("queue=16", "queue=8"),
            ("stage=write", "stage=reopen"),
            ("crc32=aaa29e86", "crc32=00000000"),
            ("bytes=6000", "bytes=5999"),
            ("poisoned=1", "poisoned=0"),
            ("reset=1", "reset=0"),
            ("sampled_remaining=24576", "sampled_remaining=16383"),
            ("sampled_remaining=24576", "sampled_remaining=131073"),
            ("sampled_remaining=24576", "sampled_remaining=024576"),
            ("sampled_remaining=24576", "sampled_remaining=-1"),
            (
                "sampled_remaining=24576",
                "sampled_remaining=24576 sampled_remaining=24576",
            ),
            ("stack_bytes=131072", "stack_bytes=262144"),
        ] {
            assert!(
                !serial_matches(markers, &serial.replace(from, to)),
                "accepted {to}"
            );
        }
        for marker in markers.iter().filter(|m| m.starts_with("DOOR_P06 ")) {
            assert!(!serial_matches(markers, &serial.replace(marker, "")));
        }
        assert!(!serial_matches(
            markers,
            &format!("{serial}\nDOOR_P06 DEVICE FAIL")
        ));
        assert!(crate::scenarios::cases("P18").is_err());
        for boundary in [16384, 131072] {
            assert!(serial_matches(
                markers,
                &serial.replace(
                    "sampled_remaining=24576",
                    &format!("sampled_remaining={boundary}")
                )
            ));
        }
        assert!(!serial_matches(
            markers,
            &format!("{serial}\nDOOR_P06 STACK PASS sampled_remaining=24576 stack_bytes=131072")
        ));
    }
    #[test]
    fn writable_arguments_require_pinned_profile_and_exact_raw_transport() {
        let mut profile: Value = serde_json::from_str(include_str!(
            "../../../../profiles/hardware/qemu-storage-x64.json"
        ))
        .unwrap();
        let args =
            crate::emulator::storage_arguments(&profile, Path::new("data,with space.raw")).unwrap();
        assert_eq!(
            args,
            [
                "-drive",
                "if=none,id=door-data,format=raw,file=data,,with space.raw",
                "-device",
                "virtio-blk-pci,drive=door-data,disable-legacy=on,serial=DOOR-P06-DATA"
            ]
        );
        profile["data_disk"]["writable_targets_must_be_workspace_local"] = json!(false);
        assert!(crate::emulator::storage_arguments(&profile, Path::new("data.raw")).is_err());
    }
    #[test]
    fn raw_reopen_allows_only_the_single_reset_probe_sector() {
        let a = vec![0; DISK_BYTES];
        let mut b = a.clone();
        b[200000 * 512 + 511] = 1;
        assert!(immutable_except_scratch(&a, &b).is_ok());
        for at in [
            0,
            133120 * 512,
            198655 * 512,
            200000 * 512 - 1,
            200001 * 512,
            DISK_BYTES - 1,
        ] {
            b[at] ^= 1;
            assert!(immutable_except_scratch(&a, &b).is_err());
            b[at] ^= 1;
        }
    }
    #[test]
    fn storage_evidence_requires_each_source_and_rejects_missing_or_swapped_boots() {
        let sources = crate::evidence::snapshot(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .unwrap(),
        )
        .unwrap();
        let good = json!({"certified_phase":"P08","source_snapshot":sources});
        validate_sources(&good).unwrap();
        for name in [
            "tools/xtask/src/storage/audit.rs",
            "fs/storage-core/src/storage/ext2.rs",
            "kernel/src/drivers/queue.rs",
            "profiles/hardware/qemu-storage-x64.json",
        ] {
            let mut bad = good.clone();
            bad["source_snapshot"]["files"]
                .as_array_mut()
                .unwrap()
                .retain(|f| f["path"] != name);
            assert!(validate_sources(&bad).is_err());
        }
        assert!(validate_suite(Path::new("."), &json!({}), &[]).is_err());
        let fixture = crate::test_support::Workspace::new();
        let run = Run::new(fixture.root.clone()).unwrap();
        let name = create(&run).unwrap();
        let persisted = persisted_fixture();
        fs::write(fixture.root.join(&name), &persisted).unwrap();
        let suite = json!({"storage":suite_evidence(&fixture.root,&name).unwrap()});
        let mut guests = Vec::new();
        let mut before = suite["storage"]["initial"]["audit"].clone();
        for (index, mode) in ["storage-write", "storage-reopen"].into_iter().enumerate() {
            let token = format!("boot-{index}");
            let directory =
                paths::fresh_directory(&fixture.root, &format!("out/qemu/{token}")).unwrap();
            fs::write(directory.join("data-after.raw"), &persisted).unwrap();
            let after = audit::inspect(&persisted, true).unwrap();
            guests.push(json!({"scenario":mode,"run_token":token,"data_disk":{"path":name,"before":before,"after":{"path":format!("out/qemu/{token}/data-after.raw"),"sha256":after["sha256"],"audit":after},"audit_error":null}}));
            before = after;
        }
        validate_suite(&fixture.root, &suite, &guests).unwrap();
        for mut bad in [
            Vec::new(),
            vec![guests[0].clone()],
            vec![guests[1].clone(), guests[0].clone()],
        ] {
            assert!(validate_suite(&fixture.root, &suite, &bad).is_err());
            bad.clear();
        }
        let mut bad = guests.clone();
        bad[1]["data_disk"]["before"]["sha256"] = json!("0".repeat(64));
        assert!(validate_suite(&fixture.root, &suite, &bad).is_err());
    }
}
