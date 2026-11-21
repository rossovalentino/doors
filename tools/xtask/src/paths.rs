//! Conservative confinement for trusted local tooling; not a hostile filesystem sandbox.
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

pub fn relative_path(name: &str) -> Result<&Path, String> {
    let path = Path::new(name);
    if name.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || name.contains(':')
        || name.contains('\\')
    {
        return Err(format!("invalid relative path: {name}"));
    }
    Ok(path)
}
pub fn existing(root: &Path, name: &str) -> Result<PathBuf, String> {
    let path = root.join(relative_path(name)?);
    reject_links(root, &path)?;
    let resolved = path.canonicalize().map_err(|e| format!("{name}: {e}"))?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(format!("not a confined file: {name}"));
    }
    Ok(resolved)
}
pub fn reject_links(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path.strip_prefix(root).map_err(|_| "path escapes root")?;
    let mut part = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err("non-normal path component".into());
        }
        part.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&part) {
            let mut linked = metadata.file_type().is_symlink();
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                linked |= metadata.file_attributes() & 0x400 != 0;
            }
            if linked {
                return Err(format!("link/reparse point refused: {}", part.display()));
            }
        }
    }
    Ok(())
}
pub fn fresh_directory(root: &Path, name: &str) -> Result<PathBuf, String> {
    let relative = relative_path(name)?;
    if !relative.starts_with("out") && !crate::layout::retained_directory(name) {
        return Err(
            "generated directories require out/ or a canonical retained generated run".into(),
        );
    }
    let path = root.join(relative);
    reject_links(root, &path)?;
    fs::create_dir_all(path.parent().ok_or("missing parent")?).map_err(|e| e.to_string())?;
    fs::create_dir(&path).map_err(|e| format!("exclusive destination {}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal_devices_streams_and_non_normal_paths() {
        for path in [
            "../disk",
            "/disk",
            "C:/disk",
            "\\\\.\\PhysicalDrive0",
            "out/file:stream",
            "./out/file",
            "out\\file",
            "",
        ] {
            assert!(relative_path(path).is_err(), "{path}");
        }
        assert!(relative_path("out/space dir/file").is_ok());
    }
}
