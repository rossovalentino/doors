//! Reusable P07 journal fixtures and independent retained-image verification.
pub(crate) mod catalog;
pub(crate) mod format;
pub(crate) mod journal;
pub(crate) mod large;
pub(crate) mod nodes;
pub mod scenarios;
pub mod session;
pub(crate) mod stage;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod tree;

use crate::{
    evidence::{Run, relative, write_json},
    paths,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub const PROFILE: &str = "profiles/hardware/qemu-persistence-x64.json";
pub fn validate_sources(report: &Value) -> Result<(), String> {
    let files = report["source_snapshot"]["files"]
        .as_array()
        .ok_or("P07 source identity")?;
    for source in [
        "tools/xtask/src/persistence/mod.rs",
        "tools/xtask/src/persistence/format.rs",
        "tools/xtask/src/persistence/journal.rs",
        "tools/xtask/src/persistence/tree.rs",
        "tools/xtask/src/persistence/catalog.rs",
        "tools/xtask/src/persistence/scenarios.rs",
        "tools/xtask/src/persistence/session.rs",
        "profiles/hardware/qemu-persistence-x64.json",
        "kernel/src/block_device.rs",
        "kernel/src/persistence_diagnostics.rs",
        "kernel/src/storage/journal.rs",
        "kernel/src/storage/ext2.rs",
        "kernel/src/persistence/namespace.rs",
        "kernel/src/persistence/security.rs",
        "kernel/src/persistence/path.rs",
        "kernel/src/persistence/config.rs",
        "kernel/src/persistence/codec.rs",
        "specs/contracts/storage-journal-v1.md",
        "specs/contracts/persistent-namespace-v1.md",
    ] {
        let source = crate::phase::storage_source(report, source);
        let matching: Vec<_> = files.iter().filter(|f| f["path"] == source).collect();
        require(
            matching.len() == 1
                && matching[0]["sha256"]
                    .as_str()
                    .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
            &format!("missing/duplicate P07 source identity: {source}"),
        )?;
    }
    Ok(())
}
const HOME_FIRST: u64 = 133120;
const HOME_SECTORS: u64 = 65536;
const LOG_FIRST: u64 = 202752;
const LOG_SECTORS: u64 = 4096;
pub fn is_mode(mode: &str) -> bool {
    matches!(
        mode,
        "persist-save"
            | "persist-reopen"
            | "journal-prepare"
            | "journal-commit"
            | "journal-first-home"
            | "journal-home"
            | "journal-checkpoint"
            | "recover-old"
            | "recover-new"
            | "persist-old"
            | "persist-new"
            | "persist-full"
    )
}
pub fn interruption_stage(mode: &str) -> Option<&'static str> {
    match mode {
        "journal-prepare" => Some("prepare"),
        "journal-commit" => Some("commit"),
        "journal-first-home" => Some("first_home"),
        "journal-home" => Some("home"),
        "journal-checkpoint" => Some("checkpoint"),
        _ => None,
    }
}
pub fn interrupted(serial: &str, token: &str, mode: &str) -> bool {
    let Some(stage) = interruption_stage(mode) else {
        return false;
    };
    let prefix = format!("DOOR_TEST v1 {token} ");
    let records: Vec<_> = serial
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect();
    if records != ["POST_EXIT_BOOT_SERVICES", "JOURNAL_INTERRUPTED"] {
        return false;
    }
    let prefix = format!("DOOR_P07 INTERRUPT stage={stage} generation=");
    let records: Vec<_> = serial
        .lines()
        .filter(|line| line.starts_with("DOOR_P07 INTERRUPT"))
        .collect();
    if records.len() != 1 {
        return false;
    }
    records[0]
        .strip_prefix(&prefix)
        .is_some_and(|s| s.parse::<u64>().is_ok_and(|n| n > 0 && n.to_string() == s))
}
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn word(bytes: &[u8], at: usize, width: usize) -> Result<u64, String> {
    let field = bytes
        .get(at..at.checked_add(width).ok_or("offset overflow")?)
        .ok_or("truncated field")?;
    Ok(field
        .iter()
        .enumerate()
        .fold(0, |v, (i, b)| v | (u64::from(*b) << (8 * i))))
}

pub fn create(run: &Run) -> Result<String, String> {
    create_size(run, false)
}
pub fn create_size(run: &Run, tiny: bool) -> Result<String, String> {
    let directory = paths::fresh_directory(
        &run.root,
        &format!(
            "out/storage/{}-{}",
            run.id,
            if tiny { "full" } else { "p07" }
        ),
    )?;
    let bytes = format::image(tiny);
    let record = inspect_bytes(&bytes)?;
    for name in ["initial.raw", "data.raw"] {
        fs::write(directory.join(name), &bytes).map_err(|e| e.to_string())?;
    }
    write_json(&directory.join("initial-audit.json"), &record)?;
    let path = relative(&run.root, &directory.join("data.raw"))?;
    println!("P07 persistence fixture: {path}");
    Ok(path)
}
pub fn inspect(path: &Path) -> Result<Value, String> {
    require(
        fs::metadata(path).map_err(|e| e.to_string())?.len() == crate::storage::DISK_BYTES as u64,
        "P07 disk size",
    )?;
    inspect_bytes(&fs::read(path).map_err(|e| e.to_string())?)
}
pub(crate) fn gpt(bytes: &[u8]) -> Result<(), String> {
    require(bytes.len() == crate::storage::DISK_BYTES, "P07 disk size")?;
    require(
        bytes[510..512] == [0x55, 0xaa]
            && bytes[450] == 0xee
            && word(bytes, 454, 4)? == 1
            && word(bytes, 458, 4)? == 262143
            && bytes[462..510].iter().all(|b| *b == 0),
        "P07 protective MBR",
    )?;
    for (lba, other, array) in [(1, 262143, 2), (262143, 1, 262111)] {
        let h = &bytes[lba * 512..(lba + 1) * 512];
        let entries = &bytes[array * 512..array * 512 + 16384];
        require(
            &h[..8] == b"EFI PART"
                && word(h, 8, 4)? == 0x10000
                && word(h, 12, 4)? == 92
                && word(h, 20, 4)? == 0
                && word(h, 24, 8)? == lba as u64
                && word(h, 32, 8)? == other as u64
                && word(h, 40, 8)? == 34
                && word(h, 48, 8)? == 262110
                && &h[56..72] == b"DoorP07DiskGUID0"
                && word(h, 72, 8)? == array as u64
                && word(h, 80, 4)? == 128
                && word(h, 84, 4)? == 128,
            "P07 GPT header",
        )?;
        let mut check = h[..92].to_vec();
        check[16..20].fill(0);
        require(
            u64::from(crate::storage::audit::crc32(&check)) == word(h, 16, 4)?
                && u64::from(crate::storage::audit::crc32(entries)) == word(h, 88, 4)?
                && entries == &bytes[1024..17408],
            "P07 GPT CRC/mirror",
        )?;
    }
    let kinds: [&[u8]; 3] = [
        &[
            0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26,
            0x99, 0xc7,
        ],
        &[
            0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47,
            0x7d, 0xe4,
        ],
        b"DoorRedoJournal1",
    ];
    for (index, (id, first, last)) in [
        (b"DoorP07FatGUID00", 2048, 133119),
        (
            b"DoorP07ExtGUID00",
            HOME_FIRST,
            HOME_FIRST + HOME_SECTORS - 1,
        ),
        (b"DoorP07LogGUID00", LOG_FIRST, LOG_FIRST + LOG_SECTORS - 1),
    ]
    .into_iter()
    .enumerate()
    {
        let e = &bytes[1024 + index * 128..1152 + index * 128];
        require(
            &e[..16] == kinds[index]
                && &e[16..32] == id
                && word(e, 32, 8)? == first
                && word(e, 40, 8)? == last
                && word(e, 48, 8)? == 0,
            "P07 GPT partition",
        )?;
    }
    require(
        bytes[1408..17408].iter().all(|b| *b == 0),
        "P07 unused GPT entries",
    )
}
fn inspect_bytes(bytes: &[u8]) -> Result<Value, String> {
    gpt(bytes)?;
    let fat = crate::storage::audit::fat(bytes)?;
    let journal = journal::inspect(bytes)?;
    let home = journal.replayed_home(bytes)?;
    let tree = tree::inspect(&home)?;
    let catalog = catalog::inspect(&tree)?;
    Ok(
        json!({"schema_version":1,"record_kind":"persistence_audit","bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes)),"gpt":{"partitions":3,"primary":true,"backup":true},"fat32":fat,"journal":journal.summary(),"effective_home_sha256":format!("{:x}",Sha256::digest(&home)),"ext2":tree.summary(),"catalog":catalog}),
    )
}
