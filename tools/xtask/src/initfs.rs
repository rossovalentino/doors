use crate::{
    evidence::{Run, file_hash, relative, write_json},
    paths,
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(test)]
pub fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 2048];
    let mut header = [0; 512];
    door_initfs::encode_header(&mut header, "system/boot.txt", 3, false).unwrap();
    bytes[..512].copy_from_slice(&header);
    bytes[512..515].copy_from_slice(b"OS\n");
    bytes
}

pub fn pack(
    root: &Path,
    manifest_path: &str,
    output: &Path,
    artifacts: &[(&str, PathBuf)],
) -> Result<(), String> {
    let manifest = crate::image::read_json(&paths::existing(root, manifest_path)?)?;
    if manifest["schema_version"] != 1 || manifest["record_kind"] != "initfs_inputs" {
        return Err("unsupported initfs manifest".into());
    }
    let entries = manifest["entries"]
        .as_array()
        .ok_or("initfs entries missing")?;
    if entries.is_empty() || entries.len() > door_initfs::MAX_ENTRIES {
        return Err("initfs entry count".into());
    }
    let mut archive = Vec::new();
    let mut previous = None;
    for entry in entries {
        let name = entry["destination"].as_str().ok_or("initfs destination")?;
        if previous.is_some_and(|prior: &str| prior >= name) {
            return Err("initfs destinations must be unique and sorted".into());
        }
        previous = Some(name);
        // Optional sources (the IME dictionaries) are staged only when the
        // build has them; the guest then reports the feature unavailable.
        match entry.get("optional") {
            None => (),
            Some(serde_json::Value::Bool(true)) if entry.get("artifact").is_none() => {
                let source = entry["source"].as_str().ok_or("initfs optional source")?;
                if !root.join(source).is_file() {
                    continue;
                }
            }
            Some(_) => return Err("initfs optional flag needs a true value and a source".into()),
        }
        let source = resolve_source(root, entry, artifacts)?;
        if fs::metadata(&source).map_err(|e| e.to_string())?.len() > door_initfs::MAX_FILE as u64 {
            return Err("initfs input too large".into());
        }
        let bytes = fs::read(&source).map_err(|e| e.to_string())?;
        let mut header = [0; 512];
        door_initfs::encode_header(&mut header, name, bytes.len(), false)
            .map_err(|e| format!("initfs header {e:?}"))?;
        let growth = 512 + bytes.len().div_ceil(512) * 512;
        if archive.len() + growth + 1024 > door_initfs::MAX_ARCHIVE {
            return Err("initfs archive too large".into());
        }
        archive.extend_from_slice(&header);
        archive.extend_from_slice(&bytes);
        archive.resize(archive.len().div_ceil(512) * 512, 0);
    }
    archive.resize(archive.len() + 1024, 0);
    door_initfs::Archive::parse(&archive).map_err(|e| format!("produced initfs {e:?}"))?;
    fs::write(output, archive).map_err(|e| e.to_string())
}
/// `archive` (an already packed initfs) with one more file `name`, kept in
/// sorted order: an image variant staged without packing or building
/// anything again.
pub fn with_file(archive: &[u8], name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    fn put(out: &mut Vec<u8>, path: &str, data: &[u8], directory: bool) -> Result<(), String> {
        let mut header = [0; 512];
        door_initfs::encode_header(&mut header, path, data.len(), directory)
            .map_err(|e| format!("initfs header {e:?}"))?;
        out.extend_from_slice(&header);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
        Ok(())
    }
    let parsed = door_initfs::Archive::parse(archive).map_err(|e| format!("initfs {e:?}"))?;
    if parsed.find(name).is_some() {
        return Err(format!("initfs already holds {name}"));
    }
    let mut out = Vec::with_capacity(archive.len() + 1024 + bytes.len());
    let mut pending = Some((name, bytes));
    for index in 0..parsed.len() {
        let entry = parsed.entry(index).map_err(|e| format!("initfs {e:?}"))?;
        if let Some((n, b)) = pending.filter(|(n, _)| *n < entry.path) {
            put(&mut out, n, b, false)?;
            pending = None;
        }
        put(&mut out, entry.path, entry.data, entry.directory)?;
    }
    if let Some((n, b)) = pending {
        put(&mut out, n, b, false)?;
    }
    out.resize(out.len() + 1024, 0);
    if out.len() > door_initfs::MAX_ARCHIVE {
        return Err("initfs archive too large".into());
    }
    door_initfs::Archive::parse(&out).map_err(|e| format!("produced initfs {e:?}"))?;
    Ok(out)
}
/// `archive` with files added, or replaced where named in `replace`, kept in
/// sorted order: the P13 Windows GUI image variant stages its fixtures, GUI
/// DLLs and an extended install manifest without repacking a profile.
/// Replacing an unlisted existing file is refused.
pub fn with_files(
    archive: &[u8],
    edits: &[(&str, &[u8])],
    replace: &[&str],
) -> Result<Vec<u8>, String> {
    let parsed = door_initfs::Archive::parse(archive).map_err(|e| format!("initfs {e:?}"))?;
    let mut entries = std::collections::BTreeMap::new();
    for index in 0..parsed.len() {
        let entry = parsed.entry(index).map_err(|e| format!("initfs {e:?}"))?;
        entries.insert(
            entry.path.to_owned(),
            (entry.data.to_vec(), entry.directory),
        );
    }
    for (name, bytes) in edits {
        let existing = entries.insert((*name).to_owned(), (bytes.to_vec(), false));
        if existing.is_some() != replace.contains(name) {
            return Err(format!(
                "initfs {name}: {}",
                if existing.is_some() {
                    "already present and not marked for replacement"
                } else {
                    "marked for replacement but absent"
                }
            ));
        }
    }
    let mut out =
        Vec::with_capacity(archive.len() + edits.iter().map(|e| e.1.len() + 1024).sum::<usize>());
    for (path, (data, directory)) in &entries {
        let mut header = [0; 512];
        door_initfs::encode_header(&mut header, path, data.len(), *directory)
            .map_err(|e| format!("initfs header {e:?}"))?;
        out.extend_from_slice(&header);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    if out.len() > door_initfs::MAX_ARCHIVE {
        return Err("initfs archive too large".into());
    }
    door_initfs::Archive::parse(&out).map_err(|e| format!("produced initfs {e:?}"))?;
    Ok(out)
}
fn resolve_source(
    root: &Path,
    entry: &serde_json::Value,
    artifacts: &[(&str, PathBuf)],
) -> Result<PathBuf, String> {
    match (entry.get("source"), entry.get("artifact")) {
        (Some(source), None) => paths::existing(root, source.as_str().ok_or("initfs source")?),
        (None, Some(artifact)) => {
            let name = artifact.as_str().ok_or("initfs artifact name")?;
            let mut matches = artifacts.iter().filter(|(id, _)| *id == name);
            let (_, path) = matches
                .next()
                .ok_or("initfs artifact was not built in this pass")?;
            if matches.next().is_some() {
                return Err("duplicate built initfs artifact".into());
            }
            let relative = relative(root, path)?;
            let path = paths::existing(root, &relative)?;
            let class = entry
                .get("class")
                .map_or(Some("native-user"), serde_json::Value::as_str)
                .ok_or("initfs artifact class")?;
            // Generated application resources: validated as a pack, never
            // inspected (or accepted) as an executable.
            if class == "resource-pack" {
                if name != crate::app_assets::NAME {
                    return Err("unsupported generated resource pack".into());
                }
                crate::app_assets::validate(&fs::read(&path).map_err(|e| e.to_string())?)?;
                return Ok(path);
            }
            if !matches!(class, "native-user" | "windows-dll" | "windows-fixture") {
                return Err("unsupported initfs executable class".into());
            }
            crate::artifact::inspect(&path, class)?;
            Ok(path)
        }
        _ => Err("initfs entry needs exactly one source or built artifact".into()),
    }
}
pub fn record(run: &Run, path: &Path, manifest: &str) -> Result<(), String> {
    write_json(
        &run.directory.join("initfs.json"),
        &json!({"path":relative(&run.root,path)?,"sha256":file_hash(path)?,"format":"door-ustar-v1","inputs":manifest}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_variant_adds_one_file_in_order_and_keeps_every_other_byte() {
        let base = fixture();
        let flag = b"assist-diagnostics-v1
";
        let out = with_file(&base, "system/assist-diagnostics.conf", flag).unwrap();
        let parsed = door_initfs::Archive::parse(&out).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed.entry(0).unwrap().path,
            "system/assist-diagnostics.conf"
        );
        assert_eq!(
            parsed.file("system/assist-diagnostics.conf"),
            Some(&flag[..])
        );
        assert_eq!(
            parsed.file("system/boot.txt"),
            Some(
                &b"OS
"[..]
            )
        );
        let late = with_file(&base, "zz/last.conf", b"x").unwrap();
        let late = door_initfs::Archive::parse(&late).unwrap();
        assert_eq!(late.entry(1).unwrap().path, "zz/last.conf");
        assert!(
            with_file(&out, "system/boot.txt", b"y").is_err(),
            "no duplicates"
        );
    }

    #[test]
    fn optional_sources_are_packed_only_when_present() {
        let workspace = crate::test_support::Workspace::new();
        fs::write(workspace.root.join("boot.txt"), b"OS\n").unwrap();
        let manifest = json!({"schema_version":1,"record_kind":"initfs_inputs","entries":[
            {"source":"missing.dict","destination":"ime/a.dict","optional":true},
            {"source":"boot.txt","destination":"system/boot.txt"}]});
        fs::write(
            workspace.root.join("m.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let out = workspace.root.join("o.tar");
        pack(&workspace.root, "m.json", &out, &[]).unwrap();
        let bytes = fs::read(&out).unwrap();
        let archive = door_initfs::Archive::parse(&bytes).unwrap();
        assert!(archive.file("ime/a.dict").is_none() && archive.file("system/boot.txt").is_some());
        fs::write(workspace.root.join("missing.dict"), b"x").unwrap();
        pack(&workspace.root, "m.json", &out, &[]).unwrap();
        let bytes = fs::read(&out).unwrap();
        assert_eq!(
            door_initfs::Archive::parse(&bytes)
                .unwrap()
                .file("ime/a.dict"),
            Some(&b"x"[..])
        );
        let bad = json!({"schema_version":1,"record_kind":"initfs_inputs","entries":[
            {"source":"boot.txt","destination":"system/boot.txt","optional":"yes"}]});
        fs::write(
            workspace.root.join("m.json"),
            serde_json::to_vec(&bad).unwrap(),
        )
        .unwrap();
        assert!(pack(&workspace.root, "m.json", &out, &[]).is_err());
    }

    #[test]
    fn generated_native_inputs_are_inspected_and_pack_exact_bytes_deterministically() {
        let workspace = crate::test_support::Workspace::new();
        let path = workspace.root.join("user.elf");
        let mut bytes = crate::image_fixtures::elf();
        crate::image_fixtures::put64(&mut bytes, 24, 0x400000);
        crate::image_fixtures::put64(&mut bytes, 80, 0x400000);
        fs::write(&path, &bytes).unwrap();
        let manifest = json!({"schema_version":1,"record_kind":"initfs_inputs","entries":[{"artifact":"native-runtime","destination":"bin/native-runtime.elf"}]});
        fs::write(
            workspace.root.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let a = workspace.root.join("a.tar");
        let b = workspace.root.join("b.tar");
        pack(
            &workspace.root,
            "manifest.json",
            &a,
            &[("native-runtime", path.clone())],
        )
        .unwrap();
        pack(
            &workspace.root,
            "manifest.json",
            &b,
            &[("native-runtime", path)],
        )
        .unwrap();
        let packed = fs::read(a).unwrap();
        assert_eq!(packed, fs::read(b).unwrap());
        assert_eq!(
            door_initfs::Archive::parse(&packed)
                .unwrap()
                .file("bin/native-runtime.elf"),
            Some(bytes.as_slice())
        );
    }

    #[test]
    fn generated_inputs_reject_unknown_duplicate_ambiguous_and_foreign_sources() {
        let workspace = crate::test_support::Workspace::new();
        let entry = json!({"artifact":"native-runtime"});
        assert!(resolve_source(&workspace.root, &entry, &[]).is_err());
        let path = workspace.root.join("fixture.elf");
        assert!(
            resolve_source(
                &workspace.root,
                &entry,
                &[
                    ("native-runtime", path.clone()),
                    ("native-runtime", path.clone())
                ]
            )
            .is_err()
        );
        assert!(
            resolve_source(
                &workspace.root,
                &json!({"source":"fixture.elf", "artifact":"native-runtime"}),
                &[]
            )
            .is_err()
        );
        assert!(resolve_source(&workspace.root, &json!({"source":"../escape"}), &[]).is_err());
        fs::write(&path, crate::image_fixtures::elf()).unwrap();
        // A kernel ELF with a plausible generated name cannot masquerade as userspace.
        assert!(resolve_source(&workspace.root, &entry, &[("native-runtime", path)]).is_err());
    }

    #[test]
    fn generated_resource_packs_are_validated_and_never_accepted_as_executables() {
        let workspace = crate::test_support::Workspace::new();
        let entries: [(&str, &[u8]); 1] = [("icons/a.mask", b"A")];
        let mut pack = vec![0; door_initfs::app_assets::encoded_len(&entries).unwrap()];
        door_initfs::app_assets::encode(&entries, &mut pack).unwrap();
        let path = workspace.root.join("app-assets.pack");
        fs::write(&path, &pack).unwrap();
        let entry = json!({"artifact":"app-assets","class":"resource-pack"});
        assert!(resolve_source(&workspace.root, &entry, &[("app-assets", path.clone())]).is_ok());
        // Another generated name cannot use the resource class...
        let other = json!({"artifact":"door-files","class":"resource-pack"});
        assert!(resolve_source(&workspace.root, &other, &[("door-files", path.clone())]).is_err());
        // ...and the pack is no executable.
        let native = json!({"artifact":"app-assets"});
        assert!(resolve_source(&workspace.root, &native, &[("app-assets", path.clone())]).is_err());
        let elf = workspace.root.join("fake.pack");
        fs::write(&elf, crate::image_fixtures::elf()).unwrap();
        assert!(resolve_source(&workspace.root, &entry, &[("app-assets", elf)]).is_err());
    }
    #[test]
    fn with_files_adds_and_replaces_only_declared_entries() {
        let base = fixture();
        let out = with_files(
            &base,
            &[("seed/a.exe", b"MZ"), ("system/boot.txt", b"X\n")],
            &["system/boot.txt"],
        )
        .unwrap();
        let parsed = door_initfs::Archive::parse(&out).unwrap();
        assert_eq!(parsed.find("seed/a.exe").unwrap().data, b"MZ");
        assert_eq!(parsed.find("system/boot.txt").unwrap().data, b"X\n");
        assert!(with_files(&base, &[("system/boot.txt", b"Y")], &[]).is_err());
        assert!(with_files(&base, &[("seed/b", b"Y")], &["seed/b"]).is_err());
    }
}
