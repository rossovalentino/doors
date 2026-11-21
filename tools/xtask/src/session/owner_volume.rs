//! Explicit first-run initialization. Existing/partial owner data is never formatted.
use crate::{
    cache,
    evidence::{Run, file_hash, relative},
    paths, persistence, storage,
};
use serde_json::json;
use sha2::Digest;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

pub fn initialize(run: &mut Run) -> Result<(), String> {
    initialize_when_idle(run, cache::owner_initialization_quiescent)
}
fn initialize_when_idle(
    run: &mut Run,
    mut quiescent: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    quiescent()?;
    cache::admit_owner_initialization(run)?;
    let directory = run.root.join("out/session");
    paths::reject_links(&run.root, &directory)?;
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    paths::reject_links(&run.root, &directory)?;
    let canonical = directory.canonicalize().map_err(|e| e.to_string())?;
    if canonical != directory || !canonical.starts_with(&run.root) {
        return Err("Owner volume parent must be the canonical workspace out/session".into());
    }
    let destination = directory.join("user.raw");
    paths::reject_links(&run.root, &destination)?;
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err("Owner volume already exists (including partial or unknown data); initialization refuses to replace it".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.to_string()),
    }
    let staged = directory.join(format!(".initialize-{}.raw", run.id));
    paths::reject_links(&run.root, &staged)?;
    // Create exclusively before assuming ownership of cleanup. A stale or
    // unrelated same-name path is neither truncated nor deleted.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .map_err(|e| format!("Exclusive owner staging: {e}"))?;
    let result = (|| {
        let bytes = persistence::format::image(false);
        if bytes.len() != storage::DISK_BYTES {
            return Err("Owner formatter size".into());
        }
        let expected = format!("{:x}", sha2::Sha256::digest(&bytes));
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(bytes);
        let audit = audit_staged(&staged, &expected)?;
        quiescent()?;
        paths::reject_links(&run.root, &destination)?;
        // Same-volume hard-link publication is atomic and fails if ANY target
        // exists. Unlike rename on Unix, it never replaces an owner/racing file.
        publish(&staged, &destination)?;
        run.check(
            "owner-volume-initialized",
            json!({"path":relative(&run.root,&destination)?,
            "bytes":storage::DISK_BYTES,"sha256":expected,"audit":audit,
            "published_without_replace":true,"data_synced_before_publish":true,
            "scope":"New persistent owner volume only; no guest/regression execution"}),
        );
        Ok(())
    })();
    drop(file);
    let cleanup = fs::remove_file(&staged);
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => Err(format!(
            "{error}; private staging cleanup also failed: {cleanup}"
        )),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(format!(
            "Owner volume was published, but its private staging cleanup failed: {error}; do not reinitialize"
        )),
        (Ok(()), Ok(())) => {
            println!(
                "Created out/session/user.raw (128MiB). Keep this file to preserve your data. Run cargo xtask desktop."
            );
            Ok(())
        }
    }
}
fn audit_staged(path: &Path, expected: &str) -> Result<serde_json::Value, String> {
    let audit = persistence::inspect(path)?;
    if audit["sha256"] != expected || file_hash(path)? != expected {
        return Err("Staged owner bytes differ from formatter".into());
    }
    Ok(audit)
}
fn publish(staged: &Path, destination: &Path) -> Result<(), String> {
    fs::hard_link(staged, destination)
        .map_err(|e| format!("Owner volume publication refused; existing data preserved: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initializes_only_new_private_volume_and_rejects_reinitialization() {
        let workspace = crate::test_support::Workspace::new();
        let mut run = Run::new(workspace.root.clone()).unwrap();
        initialize_when_idle(&mut run, || Ok(())).unwrap();
        let disk = workspace.root.join("out/session/user.raw");
        let before = file_hash(&disk).unwrap();
        let audit = persistence::inspect(&disk).unwrap();
        assert_eq!(audit["bytes"], storage::DISK_BYTES);
        assert_eq!(audit["sha256"], before);
        assert!(
            initialize_when_idle(&mut run, || Ok(()))
                .unwrap_err()
                .contains("already exists")
        );
        assert_eq!(file_hash(&disk).unwrap(), before);
        assert_eq!(fs::read_dir(disk.parent().unwrap()).unwrap().count(), 1);
    }
    #[test]
    fn racing_or_partial_owner_and_unknown_staging_are_never_replaced() {
        let workspace = crate::test_support::Workspace::new();
        let mut run = Run::new(workspace.root.clone()).unwrap();
        let directory = workspace.root.join("out/session");
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("user.raw");
        fs::write(&target, b"owner data").unwrap();
        let staged = directory.join(format!(".initialize-{}.raw", run.id));
        fs::write(&staged, b"unknown staging").unwrap();
        assert!(publish(&staged, &target).is_err());
        assert!(initialize_when_idle(&mut run, || Ok(())).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"owner data");
        fs::remove_file(&target).unwrap();
        assert!(initialize_when_idle(&mut run, || Ok(())).is_err());
        assert!(!target.exists());
        assert_eq!(fs::read(&staged).unwrap(), b"unknown staging");
    }
    #[test]
    fn corrupt_staging_cannot_reach_publication() {
        let workspace = crate::test_support::Workspace::new();
        let staged = workspace.root.join("bad.raw");
        fs::write(&staged, b"not a volume").unwrap();
        assert!(audit_staged(&staged, &"0".repeat(64)).is_err());
    }
    #[test]
    fn uncertain_or_live_process_state_prevents_any_owner_write() {
        let workspace = crate::test_support::Workspace::new();
        let mut run = Run::new(workspace.root.clone()).unwrap();
        assert!(initialize_when_idle(&mut run, || Err("process state uncertain".into())).is_err());
        assert!(!workspace.root.join("out").exists());
    }
    #[test]
    fn late_busy_or_racing_owner_preserves_target_and_cleans_owned_staging() {
        for racing_owner in [false, true] {
            let workspace = crate::test_support::Workspace::new();
            let mut run = Run::new(workspace.root.clone()).unwrap();
            let directory = workspace.root.join("out/session");
            let destination = directory.join("user.raw");
            let mut observations = 0;
            let result = initialize_when_idle(&mut run, || {
                observations += 1;
                if observations == 2 {
                    if racing_owner {
                        fs::write(&destination, b"racing owner data").unwrap();
                    } else {
                        return Err("guest became active after staging".into());
                    }
                }
                Ok(())
            });
            assert!(result.is_err());
            assert_eq!(observations, 2);
            if racing_owner {
                assert_eq!(fs::read(&destination).unwrap(), b"racing owner data");
            } else {
                assert!(!destination.exists());
            }
            assert_eq!(
                fs::read_dir(directory).unwrap().count(),
                usize::from(racing_owner)
            );
        }
    }
}
