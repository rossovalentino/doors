//! Application resource pack (ADR 0018): `profiles/build/app-assets.txt`
//! lists the `assets/` files desktop applications load at run time. The pack
//! is generated per build from those exact files, so it is reproducible from
//! the source snapshot and never checked in.
use crate::evidence::{file_hash, relative};
use door_initfs::app_assets::{Pack, encode, encoded_len, valid_key};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub const MANIFEST: &str = "profiles/build/app-assets.txt";
pub const NAME: &str = "app-assets";
pub const DESTINATION: &str = "system/app-assets.pack";

/// Validated manifest keys: sorted, unique, safe relative paths.
pub fn keys(root: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(root.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let keys: Vec<String> = text.lines().map(str::to_owned).collect();
    if keys.is_empty() {
        return Err("empty application asset manifest".into());
    }
    for (i, key) in keys.iter().enumerate() {
        if !valid_key(key.as_bytes()) {
            return Err(format!("invalid application asset key {key:?}"));
        }
        if i > 0 && keys[i - 1].as_bytes() >= key.as_bytes() {
            return Err(format!("application asset manifest unsorted at {key:?}"));
        }
    }
    Ok(keys)
}
/// Pack bytes for the manifest, read from the repository `assets/` tree.
pub fn encode_pack(root: &Path) -> Result<Vec<u8>, String> {
    let keys = keys(root)?;
    let files = keys
        .iter()
        .map(|key| {
            let path = root.join("assets").join(key);
            fs::read(&path).map_err(|e| format!("application asset {key}: {e}"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let entries: Vec<(&str, &[u8])> = keys
        .iter()
        .zip(&files)
        .map(|(k, d)| (k.as_str(), d.as_slice()))
        .collect();
    let length = encoded_len(&entries).ok_or("application asset pack exceeds its limits")?;
    if length > door_initfs::MAX_FILE {
        return Err("application asset pack exceeds the initfs file limit".into());
    }
    let mut bytes = vec![0; length];
    encode(&entries, &mut bytes).ok_or("application asset encoding")?;
    validate(&bytes)?;
    Ok(bytes)
}
pub fn validate(bytes: &[u8]) -> Result<(), String> {
    Pack::parse(bytes)
        .filter(|pack| !pack.is_empty())
        .map(|_| ())
        .ok_or_else(|| "invalid application asset pack".into())
}
/// Write the pack into a build directory and describe it like a built
/// artifact, so initfs packing and rebuild comparison treat it uniformly.
pub fn build(root: &Path, directory: &Path) -> Result<Value, String> {
    let bytes = encode_pack(root)?;
    let path = directory.join("app-assets.pack");
    fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    let entries = Pack::parse(&bytes).map_or(0, |p| p.len());
    let manifest = root.join(MANIFEST);
    Ok(json!({
        "name": NAME,
        "path": relative(root, &path)?,
        "sha256": file_hash(&path)?,
        "bytes": bytes.len(),
        "entries": entries,
        "initfs_destination": DESTINATION,
        "manifest": {"path": MANIFEST, "sha256": file_hash(&manifest)?},
        "runtime": "shared read-only application resource section",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }
    #[test]
    fn repository_manifest_names_existing_assets_and_packs_within_the_section() {
        let root = root();
        let keys = keys(&root).unwrap();
        assert!(keys.iter().any(|k| k == "fonts/segoe-ui/hinted-1x.dfa"));
        assert!(
            keys.iter()
                .any(|k| k == "fonts/cascadia-mono/CascadiaMono.ttf")
        );
        let bytes = encode_pack(&root).unwrap();
        let pack = Pack::parse(&bytes).unwrap();
        assert_eq!(pack.len(), keys.len());
        assert!(bytes.len() <= door_initfs::app_assets::MAX_BYTES);
        let mask = pack.get("icons/windows-shell/app-add.20.mask").unwrap();
        assert_eq!(
            mask,
            fs::read(root.join("assets/icons/windows-shell/app-add.20.mask")).unwrap()
        );
        // Deterministic: the same inputs give the same bytes.
        assert_eq!(bytes, encode_pack(&root).unwrap());
    }
    #[test]
    fn manifest_violations_and_foreign_packs_are_rejected() {
        let workspace = crate::test_support::Workspace::new();
        let write = |text: &str| {
            fs::create_dir_all(workspace.root.join("profiles/build")).unwrap();
            fs::write(workspace.root.join(MANIFEST), text).unwrap();
        };
        fs::create_dir_all(workspace.root.join("assets/icons")).unwrap();
        fs::write(workspace.root.join("assets/icons/a.mask"), [1, 2]).unwrap();
        fs::write(workspace.root.join("assets/icons/b.mask"), [3]).unwrap();
        write("icons/a.mask\nicons/b.mask\n");
        let bytes = encode_pack(&workspace.root).unwrap();
        assert_eq!(
            Pack::parse(&bytes).unwrap().get("icons/b.mask"),
            Some(&[3][..])
        );
        for bad in [
            "icons/b.mask\nicons/a.mask\n",
            "icons/a.mask\nicons/a.mask\n",
            "../secret\n",
            "icons/missing.mask\n",
            "",
        ] {
            write(bad);
            assert!(encode_pack(&workspace.root).is_err(), "{bad:?}");
        }
        assert!(validate(b"DOORAST1").is_err());
        assert!(validate(&crate::initfs::fixture()).is_err());
    }
    #[test]
    fn every_on_demand_resource_is_packed_into_both_desktop_archives() {
        let root = root();
        for profile in [
            "profiles/build/initfs-desktop.json",
            "profiles/build/initfs-desktop-test.json",
        ] {
            let manifest = crate::image::read_json(&root.join(profile)).unwrap();
            let entries = manifest["entries"].as_array().unwrap();
            for path in door_abi::platform::resource::PATHS {
                let entry = entries.iter().find(|e| e["destination"] == path);
                let source = entry.and_then(|e| e["source"].as_str());
                // IME dictionaries are optional, desktop archive only (the
                // window-test image runs no IME).
                if path.starts_with("ime/") {
                    let desktop = profile.ends_with("initfs-desktop.json");
                    assert_eq!(entry.is_some(), desktop, "{profile}: {path}");
                    assert!(
                        entry.is_none_or(|e| e["optional"] == true),
                        "{profile}: {path}"
                    );
                    continue;
                }
                assert!(
                    source.is_some_and(|s| root.join(s).is_file()),
                    "{profile}: {path}"
                );
            }
        }
    }
}
