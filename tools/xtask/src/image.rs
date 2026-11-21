use crate::{
    artifact,
    evidence::{Run, file_hash, relative, write_json},
    paths,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const ENTRIES: [(&str, &str); 3] = [
    ("firmware", "esp/EFI/BOOT/BOOTX64.EFI"),
    ("kernel", "esp/Door/kernel.elf"),
    ("initfs", "esp/Door/initfs.tar"),
];
pub(crate) fn inspect_role(path: &Path, role: &str) -> Result<Value, String> {
    if role != "initfs" {
        return artifact::inspect(path, role);
    }
    let length = fs::metadata(path).map_err(|e| e.to_string())?.len();
    if length == 0 || length > door_initfs::MAX_ARCHIVE as u64 {
        return Err("initfs must be nonempty and <=256 MiB".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let archive =
        door_initfs::Archive::parse(&bytes).map_err(|e| format!("initfs validation: {e:?}"))?;
    if archive.file("system/boot.txt").is_none() {
        return Err("initfs missing regular system/boot.txt".into());
    }
    Ok(
        json!({"class":"initfs-archive","sha256":file_hash(path)?,"file_bytes":length,"archive_semantics":"door-ustar-v1","entries":archive.len()}),
    )
}
pub fn read_json(path: &Path) -> Result<Value, String> {
    // This reader also handles cumulative evidence with independently bound
    // artifact snapshots. Each artifact binds the full source manifest (about
    // 5,000 files), so the P13 cumulative report (40 boot artifacts plus the
    // desktop and Win32 builds) is about 46 MB. Guest formats retain their
    // separate, much smaller checked format bounds. With the P17 32-bit
    // builds (23 more artifacts) the desktop record passed 128 MiB
    // (measured 11 November 2026); a record that names the snapshot once
    // instead of per artifact is planned.
    const LIMIT: u64 = 512 * 1024 * 1024;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > LIMIT {
        return Err("JSON input exceeds 512 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("JSON input exceeds 512 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
pub fn validate_inputs(
    root: &Path,
    manifest: &Value,
) -> Result<Vec<(PathBuf, &'static str, Value)>, String> {
    if manifest["schema_version"] != 1 || manifest["record_kind"] != "image_inputs" {
        return Err("unsupported image input schema".into());
    }
    let entries = manifest["entries"]
        .as_array()
        .ok_or("image entries missing")?;
    if entries.len() != 3 {
        return Err("exactly firmware, kernel and initfs inputs required".into());
    }
    let mut result = Vec::new();
    for (role, destination) in ENTRIES {
        let matches: Vec<_> = entries
            .iter()
            .filter(|entry| entry["role"] == role)
            .collect();
        if matches.len() != 1 {
            return Err(format!("missing/duplicate role {role}"));
        }
        let entry = matches[0];
        let input = paths::existing(root, entry["path"].as_str().ok_or("input path missing")?)?;
        if entry["sha256"] != file_hash(&input)? {
            return Err(format!("{role} input hash mismatch"));
        }
        let metadata = inspect_role(&input, role)?;
        result.push((input, destination, metadata));
    }
    Ok(result)
}
pub fn stage(run: &mut Run, manifest_path: &str) -> Result<(), String> {
    let manifest = read_json(&paths::existing(&run.root, manifest_path)?)?;
    let inputs = validate_inputs(&run.root, &manifest)?;
    let output = paths::fresh_directory(&run.root, &format!("out/images/{}", run.id))?;
    let record = stage_inputs(&output, &inputs)?;
    write_json(&output.join("image.json"), &record)?;
    println!(
        "Staged read-only ESP tree: {}",
        relative(&run.root, &output)?
    );
    Ok(())
}
pub fn stage_inputs(
    output: &Path,
    inputs: &[(PathBuf, &'static str, Value)],
) -> Result<Value, String> {
    let mut entries = Vec::new();
    for (input, destination, metadata) in inputs {
        let target = output.join(destination);
        paths::reject_links(output, &target)?;
        fs::create_dir_all(target.parent().ok_or("missing parent")?).map_err(|e| e.to_string())?;
        fs::copy(input, &target).map_err(|e| e.to_string())?;
        if file_hash(&target)?
            != metadata["sha256"]
                .as_str()
                .ok_or("artifact digest absent")?
        {
            return Err("staged bytes differ from inspected input".into());
        }
        entries.push(json!({"path":destination,"sha256":metadata["sha256"],"artifact":metadata}));
    }
    Ok(
        json!({"schema_version":1,"record_kind":"staged_image","kind":"qemu-readonly-vvfat-esp-tree","entries":entries,"boot_qualified":false}),
    )
}
pub fn validate_staged(root: &Path, image_path: &str) -> Result<(PathBuf, Value), String> {
    let path = paths::existing(root, image_path)?;
    let base = path.parent().ok_or("missing image directory")?;
    let record = read_json(&path)?;
    if record["schema_version"] != 1
        || record["record_kind"] != "staged_image"
        || record["kind"] != "qemu-readonly-vvfat-esp-tree"
    {
        return Err("unsupported staged image".into());
    }
    let entries = record["entries"]
        .as_array()
        .ok_or("staged entries missing")?;
    if entries.len() != 3 {
        return Err("incomplete staged image".into());
    }
    for (role, destination) in ENTRIES {
        let matches: Vec<_> = entries
            .iter()
            .filter(|entry| entry["path"] == destination)
            .collect();
        if matches.len() != 1 {
            return Err("staged destination missing/duplicated".into());
        }
        let entry = matches[0];
        let file = paths::existing(base, destination)?;
        if entry["sha256"] != file_hash(&file)? {
            return Err("staged image hash mismatch".into());
        }
        if inspect_role(&file, role)? != entry["artifact"] {
            return Err("staged artifact inspection mismatch".into());
        }
    }
    Ok((base.to_path_buf(), record))
}

#[cfg(test)]
mod tests {
    #[test]
    fn expanded_build_receipt_is_readable_but_json_remains_bounded() {
        use std::io::{Seek, SeekFrom, Write};
        let workspace = crate::test_support::Workspace::new();
        let path = workspace.root.join("expanded.json");
        let mut file = std::fs::File::create(&path).unwrap();
        // Large whitespace is legal JSON and avoids duplicating a full receipt.
        file.write_all(b"{}").unwrap();
        let spaces = vec![b' '; 17 * 1024 * 1024];
        file.write_all(&spaces).unwrap();
        drop(file);
        assert_eq!(super::read_json(&path).unwrap(), serde_json::json!({}));
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(512 * 1024 * 1024)).unwrap();
        file.write_all(b" ").unwrap();
        drop(file);
        assert_eq!(
            super::read_json(&path).unwrap_err(),
            "JSON input exceeds 512 MiB"
        );
    }
    use super::*;
    #[test]
    fn directory_cannot_satisfy_boot_configuration_role() {
        let workspace = crate::test_support::Workspace::new();
        let mut bytes = [0; 1536];
        let mut header = [0; 512];
        door_initfs::encode_header(&mut header, "system/boot.txt", 0, true).unwrap();
        bytes[..512].copy_from_slice(&header);
        let path = workspace.root.join("directory.tar");
        fs::write(&path, bytes).unwrap();
        assert!(door_initfs::Archive::parse(&bytes).is_ok());
        assert!(inspect_role(&path, "initfs").is_err());
    }
    #[test]
    fn staging_rejects_missing_duplicate_and_escaping_inputs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        for entries in [
            json!([]),
            json!([{"role":"firmware"},{"role":"firmware"},{"role":"kernel"}]),
            json!([{"role":"firmware","path":"../x"},{"role":"kernel"},{"role":"initfs"}]),
        ] {
            assert!(
                validate_inputs(
                    &root,
                    &json!({"schema_version":1,"record_kind":"image_inputs","entries":entries})
                )
                .is_err()
            );
        }
    }

    #[test]
    fn staging_roundtrip_hashes_private_outputs_and_rejects_tampering() {
        let workspace = crate::test_support::Workspace::new();
        let root = &workspace.root;
        fs::write(root.join("firmware.efi"), crate::image_fixtures::pe()).unwrap();
        fs::write(root.join("kernel.elf"), crate::image_fixtures::elf()).unwrap();
        fs::write(root.join("initfs.tar"), crate::initfs::fixture()).unwrap();
        let entries:Vec<_>=[("firmware","firmware.efi"),("kernel","kernel.elf"),("initfs","initfs.tar")].into_iter()
            .map(|(role,path)|json!({"role":role,"path":path,"sha256":file_hash(&root.join(path)).unwrap()})).collect();
        let manifest = json!({"schema_version":1,"record_kind":"image_inputs","entries":entries});
        let inputs = validate_inputs(root, &manifest).unwrap();
        let first = paths::fresh_directory(root, "out/images/first").unwrap();
        let second = paths::fresh_directory(root, "out/images/second").unwrap();
        let record = stage_inputs(&first, &inputs).unwrap();
        assert_eq!(record, stage_inputs(&second, &inputs).unwrap());
        write_json(&first.join("image.json"), &record).unwrap();
        assert!(validate_staged(root, "out/images/first/image.json").is_ok());
        let mut altered = record.clone();
        altered["entries"][2]["artifact"]["file_bytes"] = json!(0);
        write_json(&first.join("image.json"), &altered).unwrap();
        assert!(validate_staged(root, "out/images/first/image.json").is_err());
        fs::write(first.join("esp/Door/initfs.tar"), b"").unwrap();
        altered["entries"][2]["sha256"] =
            json!(file_hash(&first.join("esp/Door/initfs.tar")).unwrap());
        altered["entries"][2]["artifact"]["sha256"] = altered["entries"][2]["sha256"].clone();
        write_json(&first.join("image.json"), &altered).unwrap();
        assert!(validate_staged(root, "out/images/first/image.json").is_err());
        fs::write(first.join("esp/Door/initfs.tar"), crate::initfs::fixture()).unwrap();
        write_json(&first.join("image.json"), &record).unwrap();
        assert!(paths::fresh_directory(root, "out/images/first").is_err());
        fs::write(first.join("esp/Door/kernel.elf"), b"corrupt").unwrap();
        assert!(validate_staged(root, "out/images/first/image.json").is_err());
        fs::write(root.join("firmware.efi"), b"corrupt").unwrap();
        assert!(validate_inputs(root, &manifest).is_err());
    }
}
