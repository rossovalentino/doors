//! Explicit local reference acquisition. Resource DLLs are read as bytes only.
//! Export/check subsequently uses pinned files without consulting the host OS.
use crate::evidence::{file_hash, write_json};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};

const MAX_ICON: usize = 4 * 1024 * 1024;
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("icon source exceeds byte bound".into());
    }
    Ok(bytes)
}
pub(super) struct Frame {
    pub(super) directory: [u8; 8],
    pub(super) bytes: Vec<u8>,
}
pub(super) fn archive(frames: &[Frame]) -> Result<Vec<u8>, String> {
    if frames.is_empty() || frames.len() > 64 {
        return Err("icon frame count".into());
    }
    let mut offset = 6 + frames.len() * 16;
    let mut result = vec![0, 0, 1, 0];
    result.extend_from_slice(&(frames.len() as u16).to_le_bytes());
    for frame in frames {
        let end = offset
            .checked_add(frame.bytes.len())
            .ok_or("icon extent overflow")?;
        if frame.directory[0] != frame.directory[1]
            || frame.directory[3] != 0
            || frame.bytes.is_empty()
            || end > MAX_ICON
        {
            return Err("icon frame bounds".into());
        }
        result.extend_from_slice(&frame.directory);
        result.extend_from_slice(&(frame.bytes.len() as u32).to_le_bytes());
        result.extend_from_slice(&(offset as u32).to_le_bytes());
        offset = end;
    }
    for frame in frames {
        result.extend_from_slice(&frame.bytes);
    }
    Ok(result)
}
pub(super) fn group_archive(
    group: &[u8],
    mut resource: impl FnMut(u16) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    if group.len() < 6 || group[..4] != [0, 0, 1, 0] {
        return Err("icon group header".into());
    }
    let count = u16::from_le_bytes(group[4..6].try_into().unwrap()) as usize;
    if count == 0 || count > 64 || group.len() != 6 + count * 14 {
        return Err("icon group directory".into());
    }
    let mut frames = Vec::new();
    let mut budget = 6 + count * 16;
    for entry in group[6..].chunks_exact(14) {
        budget = budget
            .checked_add(u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize)
            .ok_or("icon group extent overflow")?;
        if budget > MAX_ICON {
            return Err("icon group exceeds byte bound".into());
        }
    }
    for entry in group[6..].chunks_exact(14) {
        let length = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let id = u16::from_le_bytes(entry[12..14].try_into().unwrap());
        if id == 0 || length == 0 || length > MAX_ICON {
            return Err("icon resource bounds".into());
        }
        let bytes = resource(id)?;
        if bytes.len() != length {
            return Err("icon resource length differs".into());
        }
        frames.push(Frame {
            directory: entry[..8].try_into().unwrap(),
            bytes,
        });
    }
    archive(&frames)
}
fn commit(root: &Path, name: &str, bytes: Vec<u8>, sources: Value) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 48
        || !name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
    {
        return Err("icon acquisition name".into());
    }
    let dir = root.join("assets/icons/windows");
    let path = dir.join(format!("{name}.ico"));
    if path.exists() && fs::read(&path).map_err(|e| e.to_string())? != bytes {
        return Err("refusing to replace a different pinned icon".into());
    }
    // Verify at least one frame can be decoded by the actual build pipeline.
    let count = u16::from_le_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let mut decoded = 0;
    for entry in bytes[6..6 + count * 16].chunks_exact(16) {
        let start = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        let length = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        decoded += usize::from(super::ico::frame(&bytes[start..start + length])?.is_some());
    }
    if decoded == 0 {
        return Err("no supported acquired icon frame".into());
    }
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let provenance = format!("{name}.source.json");
    let manifest_path = dir.join("manifest.json");
    let mut manifest = crate::image::read_json(&manifest_path)?;
    let icons = manifest["icons"]
        .as_array_mut()
        .ok_or("Windows icon inventory")?;
    let file = format!("{name}.ico");
    if let Some(prior) = icons.iter().find(|v| v["file"] == file) {
        if prior["sha256"] != hash || prior["provenance"] != provenance {
            return Err("pinned icon manifest differs".into());
        }
    } else {
        icons.push(
            json!({"file":file,"source":format!("installed Windows reference; {provenance}"),
            "provenance":provenance,"sha256":hash}),
        );
    }
    // All identity checks precede writes, including the missing-ICO/existing-pin
    // case. Interrupted writes cannot silently change an existing source pin.
    fs::write(&path, bytes).map_err(|e| e.to_string())?;
    write_json(
        &dir.join(&provenance),
        &json!({"schema_version":1,
        "kind":"installed_windows_icon_reference","file":file,"sha256":hash,
        "sources":sources,"frames":count,"supported_frames":decoded,
        "use":"Owner-provided installed Windows reference; local development"}),
    )?;
    write_json(&manifest_path, &manifest)?;
    println!("Acquired {name}: {count} authored frames ({decoded} supported); {hash}");
    Ok(())
}
pub fn pngs(
    root: &Path,
    name: &str,
    directory: &str,
    stem: &str,
    suffix: &str,
) -> Result<(), String> {
    let dir = Path::new(directory)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !dir.is_dir()
        || stem.is_empty()
        || !stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("PNG icon source directory/stem".into());
    }
    let mut frames = Vec::new();
    let mut sources = Vec::new();
    for size in [16u32, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256] {
        let path = dir.join(format!("{stem}.targetsize-{size}{suffix}.png"));
        if !path.is_file() {
            continue;
        }
        let hash = file_hash(&path)?;
        let bytes = read_bounded(&path, MAX_ICON)?;
        let frame = super::ico::frame(&bytes)?.ok_or("PNG icon frame")?;
        if frame.width() != size
            || frame.height() != size
            || format!("{:x}", Sha256::digest(&bytes)) != hash
            || file_hash(&path)? != hash
        {
            return Err("PNG native size/source changed".into());
        }
        let n = if size == 256 { 0 } else { size as u8 };
        if frames
            .iter()
            .map(|f: &Frame| f.bytes.len() + 16)
            .sum::<usize>()
            + bytes.len()
            + 22
            > MAX_ICON
        {
            return Err("PNG icon collection exceeds byte bound".into());
        }
        frames.push(Frame {
            directory: [n, n, 0, 0, 1, 0, 32, 0],
            bytes,
        });
        sources.push(json!({"path":path.to_string_lossy(),"sha256":hash,"size":size}));
    }
    commit(
        root,
        name,
        archive(&frames)?,
        json!({"method":"authored targetsize PNGs","files":sources}),
    )
}
pub fn resource(root: &Path, name: &str, source: &str, id: u16) -> Result<(), String> {
    let path = Path::new(source)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if id == 0
        || !path.is_file()
        || fs::metadata(&path).map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024
    {
        return Err("resource module/id bounds".into());
    }
    let hash = file_hash(&path)?;
    let source_bytes = read_bounded(&path, 128 * 1024 * 1024)?;
    if format!("{:x}", Sha256::digest(&source_bytes)) != hash {
        return Err("resource bytes differ from source identity".into());
    }
    let module = super::resources::Image::parse(&source_bytes)?;
    let bytes = group_archive(&module.read(14, id)?, |id| module.read(3, id))?;
    if file_hash(&path)? != hash {
        return Err("resource module changed during acquisition".into());
    }
    commit(
        root,
        name,
        bytes,
        json!({"method":"checked PE RT_GROUP_ICON/RT_ICON bytes; no module loading",
        "module":path.to_string_lossy(),"module_sha256":hash,"group_id":id}),
    )
}
/// Prints the icon groups of a resource module (read as bytes only).
pub fn list(source: &str) -> Result<(), String> {
    let bytes = read_bounded(Path::new(source), 128 * 1024 * 1024)?;
    let module = super::resources::Image::parse(&bytes)?;
    for group in module.list(14)? {
        println!("{group}");
    }
    Ok(())
}
pub fn named_resource(root: &Path, name: &str, source: &str, group: &str) -> Result<(), String> {
    let path = Path::new(source)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if group.is_empty() || group.len() > 96 {
        return Err("icon group name bound".into());
    }
    let bytes = read_bounded(&path, 128 * 1024 * 1024)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let module = super::resources::Image::parse(&bytes)?;
    let ico = group_archive(&module.read_named(14, group)?, |id| module.read(3, id))?;
    if file_hash(&path)? != hash {
        return Err("resource module changed during acquisition".into());
    }
    commit(
        root,
        name,
        ico,
        json!({"method":"checked named PE RT_GROUP_ICON and RT_ICON bytes; no executable loading","module":path.to_string_lossy(),"module_sha256":hash,"group_name":group}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn group_conversion_preserves_authored_data_and_rejects_missing_or_wrong_resources() {
        let mut group = vec![0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0];
        group.extend_from_slice(&3u32.to_le_bytes());
        group.extend_from_slice(&7u16.to_le_bytes());
        let ico = group_archive(&group, |id| {
            assert_eq!(id, 7);
            Ok(vec![4, 5, 6])
        })
        .unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(&ico[18..22], &22u32.to_le_bytes());
        assert_eq!(&ico[22..], &[4, 5, 6]);
        assert!(group_archive(&group, |_| Ok(vec![1, 2])).is_err());
        assert!(group_archive(&group, |_| Err("missing".into())).is_err());
        for bytes in [&group[..19], &[0, 0, 1, 0, 0, 0], &[0, 0, 1, 0, 65, 0]] {
            assert!(group_archive(bytes, |_| panic!("invalid directory must not load")).is_err());
        }
        group[14..18].copy_from_slice(&(MAX_ICON as u32 + 1).to_le_bytes());
        assert!(group_archive(&group, |_| panic!("oversize must not load")).is_err());
    }
    #[test]
    fn bounded_sources_and_total_group_budget_fail_before_loading_payloads() {
        let path = std::env::temp_dir().join(format!("door-icon-bound-{}.bin", std::process::id()));
        fs::write(&path, [1, 2, 3, 4]).unwrap();
        assert_eq!(read_bounded(&path, 4).unwrap(), [1, 2, 3, 4]);
        assert!(read_bounded(&path, 3).is_err());
        fs::remove_file(&path).unwrap();
        let mut group = vec![0, 0, 1, 0, 2, 0];
        for id in [1u16, 2] {
            group.extend_from_slice(&[16, 16, 0, 0, 1, 0, 32, 0]);
            group.extend_from_slice(&((MAX_ICON / 2) as u32).to_le_bytes());
            group.extend_from_slice(&id.to_le_bytes());
        }
        assert!(
            group_archive(&group, |_| panic!("reject total extent before allocation")).is_err()
        );
    }
    #[test]
    fn mismatched_manifest_pin_does_not_write_a_missing_asset_or_provenance() {
        let root = std::env::temp_dir().join(format!("door-icon-pin-{}", std::process::id()));
        let dir = root.join("assets/icons/windows");
        fs::create_dir_all(&dir).unwrap();
        let manifest =
            json!({"icons":[{"file":"test.ico","sha256":"wrong","provenance":"test.source.json"}]});
        write_json(&dir.join("manifest.json"), &manifest).unwrap();
        let actual =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/windows/text-file.ico");
        assert!(commit(&root, "test", fs::read(actual).unwrap(), json!({})).is_err());
        assert!(!dir.join("test.ico").exists() && !dir.join("test.source.json").exists());
        assert_eq!(
            crate::image::read_json(&dir.join("manifest.json")).unwrap(),
            manifest
        );
        fs::remove_file(dir.join("manifest.json")).unwrap();
        fs::remove_dir(&dir).unwrap();
        fs::remove_dir(dir.parent().unwrap()).unwrap();
        fs::remove_dir(root.join("assets")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
