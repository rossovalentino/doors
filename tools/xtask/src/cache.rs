//! Owner-requested budget for generated out/, not target/ or retained reports.
//! Cleanup is deliberately limited to unreferenced, completed Cargo caches.
use crate::{
    evidence::{Run, relative, write_json},
    paths,
};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

/// The out/ budget. 20 GB until 13 November 2026, when the owner lifted it:
/// a full P17 gate keeps about 65 desktop sessions (a 240 MB boot-disk copy
/// each) for its own end-of-run checks and stopped at 18.7 GB.
pub const LIMIT: u64 = 32_000_000_000;
// Conservative admission reservation for a fresh double build + foundation suite.
const HEADROOM: u64 = 8_000_000_000;
// v2 desktop writes only two bounded latest-pointer files to out; allow staging
// and replacement while preserving the independent18GB live-writer threshold.
const DESKTOP_METADATA_RESERVE: u64 = 128 * 1024 * 1024;
// Initialization briefly has two names for the128MiB disk plus small receipts.
// Count both logical names even when hard-link publication shares physical bytes.
const OWNER_INITIALIZATION_RESERVE: u64 = 320 * 1024 * 1024;
pub struct Lease {
    _file: File,
}
impl Lease {
    pub fn acquire(root: &Path) -> Result<Self, String> {
        let path = root.join("target/door-output-budget.lock");
        paths::reject_links(root, &path)?;
        fs::create_dir_all(path.parent().ok_or("budget lease parent")?)
            .map_err(|e| e.to_string())?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        #[cfg(not(windows))]
        return Err("output maintenance requires the Windows exclusive workspace lease".into());
        #[cfg(windows)]
        Ok(Self {
            _file: options.open(path).map_err(|e| {
                format!("workspace output is busy (another xtask owns its lease): {e}")
            })?,
        })
    }
}
#[derive(Default)]
struct Inventory {
    bytes: u64,
    files: u64,
    directories: Vec<PathBuf>,
}
fn inventory(root: &Path, directory: &Path, result: &mut Inventory) -> Result<(), String> {
    inventory_mode(root, directory, result, false)
}
/// While a guest or build is writing, entries vanish or are briefly locked
/// (delete-pending, exclusively opened disks): a live headroom scan skips them
/// rather than stopping the writer.
fn transient(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
    )
}
fn inventory_mode(
    root: &Path,
    directory: &Path,
    result: &mut Inventory,
    live: bool,
) -> Result<(), String> {
    paths::reject_links(root, directory)?;
    if !directory.exists() {
        return Ok(());
    }
    let entries = match fs::read_dir(directory) {
        Err(error) if live && transient(&error) => return Ok(()),
        result => result.map_err(|e| e.to_string())?,
    };
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        let metadata = match fs::symlink_metadata(&path) {
            Err(error) if live && transient(&error) => continue,
            result => result.map_err(|e| e.to_string())?,
        };
        let mut linked = metadata.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            linked |= metadata.file_attributes() & 0x400 != 0;
        }
        if linked {
            return Err(format!(
                "output link/reparse point refused: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            result.directories.push(path.clone());
            inventory_mode(root, &path, result, live)?;
        } else if metadata.is_file() {
            result.bytes = result
                .bytes
                .checked_add(metadata.len())
                .ok_or("output size overflow")?;
            result.files += 1;
        } else {
            return Err("unknown output file kind".into());
        }
    }
    Ok(())
}
fn inspect(root: &Path) -> Result<Inventory, String> {
    let mut result = Inventory::default();
    inventory(root, &root.join("out"), &mut result)?;
    Ok(result)
}
pub fn status(root: &Path) -> Result<Value, String> {
    let observed = inspect(root)?;
    Ok(
        json!({"scope":"out only; excludes target and reports", "limit_bytes":LIMIT,
        "accounting":"logical file lengths (conservative for NTFS-compressed files)",
        "bytes":observed.bytes,"files":observed.files,"within_budget":observed.bytes<=LIMIT,
        "admission_headroom_bytes":HEADROOM,"owner_volume":"out/session is always protected"}),
    )
}
fn collect_references(value: &Value, references: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let text = text.replace('\\', "/").to_ascii_lowercase();
            if text.contains("out/") || text.contains("reports/runs/") {
                references.push(text);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_references(value, references);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_references(value, references);
            }
        }
        _ => (),
    }
}
fn retained_references(
    root: &Path,
    directory: &Path,
    references: &mut Vec<String>,
) -> Result<(), String> {
    paths::reject_links(root, directory)?;
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        paths::reject_links(root, &path)?;
        if path.is_dir() {
            // Disposable compiler metadata is not retained proof and must not
            // pin its own cache. External references still veto its deletion.
            if cache_owner(&relative(root, &path)?).is_some() {
                continue;
            }
            retained_references(root, &path, references)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let value = bounded_json(&path)?;
            collect_references(&value, references);
        }
    }
    Ok(())
}
// Matches `image::read_json`: cumulative reports bind full source manifests.
const MAX_JSON: u64 = 512 * 1024 * 1024;
fn bounded_json(path: &Path) -> Result<Value, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_JSON {
        return Err("oversized retained JSON prevents safe cache cleanup".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_JSON + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_JSON {
        return Err("retained JSON grew beyond cleanup limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("retained JSON {}: {e}", path.display()))
}
fn cache_owner(name: &str) -> Option<&str> {
    let parts: Vec<_> = name.split('/').collect();
    let legacy = parts.len() >= 6 && parts[..2] == ["out", "builds"];
    let retained = parts.len() >= 8
        && parts[..2] == ["reports", "runs"]
        && parts[3..5] == ["generated", "builds"];
    if !legacy && !retained {
        return None;
    }
    let owner = parts[2];
    if !canonical_owner(owner)
        || !matches!(parts[parts.len() - 2], "release" | "debug")
        || !matches!(
            parts[parts.len() - 1],
            ".fingerprint" | "deps" | "build" | "incremental" | "examples"
        )
    {
        return None;
    }
    Some(owner)
}
fn canonical_owner(name: &str) -> bool {
    crate::layout::canonical_owner(name)
}
fn sealed_owner(value: &Value, owner: &str, qemu: Option<&str>) -> bool {
    value["record_kind"] == "host_run"
        && value["id"] == owner
        && value["success"].is_boolean()
        && value["inputs_unchanged"].is_boolean()
        && value["commands"]
            .as_array()
            .is_some_and(|commands| eligible_commands(commands, qemu))
}
fn referenced(name: &str, references: &[String]) -> bool {
    let name = name.to_ascii_lowercase();
    references.iter().any(|text| {
        text.match_indices(&name).any(|(position, _)| {
            let suffix = &text[position + name.len()..];
            suffix.is_empty()
                || suffix.starts_with('/')
                || suffix.starts_with('"')
                || suffix.starts_with(',')
                || suffix.starts_with(' ')
        })
    })
}
pub(crate) fn owner_initialization_quiescent() -> Result<(), String> {
    if qemu_active()? {
        Err("Close running QEMU/build processes before initializing a new owner volume".into())
    } else {
        Ok(())
    }
}
pub(crate) fn qemu_active() -> Result<bool, String> {
    #[cfg(windows)]
    {
        use std::{
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        let output = Command::new("tasklist.exe")
            .args(["/FO", "CSV", "/NH"])
            .creation_flags(0x0800_0000)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("cannot establish guest quiescence for cache cleanup".into());
        }
        Ok(String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            let line = line.to_ascii_lowercase();
            line.starts_with("\"qemu-system-") || line.starts_with("\"rustc.exe\"")
        }))
    }
    #[cfg(not(windows))]
    Err("guest quiescence check requires Windows".into())
}
fn retired(commands: &[Value]) -> bool {
    commands.iter().all(|command| {
        command.get("cleanup_error") == Some(&Value::Null)
            && (command["lifecycle"].is_null() || command["lifecycle"] == "completed")
            && matches!(
                command["outcome"].as_str(),
                Some(
                    "passed"
                        | "failed"
                        | "timeout"
                        | "output_limit"
                        | "output_budget"
                        | "tool_error"
                )
            )
            && (command["exit_code"]
                .as_i64()
                .is_some_and(|exit| i32::try_from(exit).is_ok())
                || (command["exit_code"].is_null()
                    && command["pid"].is_null()
                    && command["spawn_error"].is_string()
                    && command["timed_out"] == false))
    })
}
fn verified_qemu(root: &Path) -> Option<String> {
    let profile = bounded_json(&root.join("profiles/hardware/qemu-diagnostic-x64.json")).ok()?;
    let name = profile["qemu"]["path"].as_str()?;
    let path = Path::new(name);
    if !path.is_absolute()
        || path.file_name()?.to_str()? != "qemu-system-x86_64.exe"
        || crate::evidence::file_hash(path).ok()?.as_str() != profile["qemu"]["sha256"].as_str()?
    {
        return None;
    }
    Some(name.to_owned())
}
fn canonical_handoff(command: &Value, qemu: &str) -> bool {
    command["argv"]
        .as_array()
        .and_then(|argv| argv.first())
        .and_then(Value::as_str)
        .is_some_and(|name| {
            name.replace('\\', "/")
                .eq_ignore_ascii_case(&qemu.replace('\\', "/"))
        })
        && command["pid"]
            .as_u64()
            .is_some_and(|pid| pid != 0 && u32::try_from(pid).is_ok())
        && command["lifecycle"] == "running"
        && command["outcome"] == "running"
        && command["required"] == false
        && command.get("exit_code") == Some(&Value::Null)
        && command["expected_exit_code"] == 0
        && command["timed_out"] == false
        && command["output_limit_exceeded"] == false
        && [
            "spawn_error",
            "controller_error",
            "cleanup_error",
            "output_budget_error",
        ]
        .iter()
        .all(|key| command.get(*key) == Some(&Value::Null))
}
fn eligible_commands(commands: &[Value], qemu: Option<&str>) -> bool {
    commands.iter().all(|command| {
        retired(std::slice::from_ref(command))
            || qemu.is_some_and(|qemu| canonical_handoff(command, qemu))
    })
}
pub fn maintain(run: &mut Run, current_complete: bool) -> Result<Value, String> {
    maintain_with(run, current_complete, qemu_active)
}
fn maintain_with(
    run: &mut Run,
    current_complete: bool,
    mut quiescence: impl FnMut() -> Result<bool, String>,
) -> Result<Value, String> {
    let before = inspect(&run.root)?;
    let qemu = verified_qemu(&run.root);
    let current_retired = current_complete && eligible_commands(run.commands(), qemu.as_deref());
    let mut completed = std::collections::BTreeSet::new();
    let mut candidate_directories = before.directories.clone();
    for build_area in ["out/builds", "reports/runs"] {
        if let Ok(entries) = fs::read_dir(run.root.join(build_area)) {
            for entry in entries {
                let owner = entry
                    .map_err(|e| e.to_string())?
                    .file_name()
                    .to_string_lossy()
                    .into_owned();
                if !canonical_owner(&owner) {
                    continue;
                }
                let retained = build_area == "reports/runs";
                if retained {
                    let generated = run
                        .root
                        .join(format!("reports/runs/{owner}/generated/builds"));
                    paths::reject_links(&run.root, &generated)?;
                    if !generated.exists() {
                        continue;
                    }
                    let mut observed = Inventory::default();
                    inventory(&run.root, &generated, &mut observed)?;
                    candidate_directories.extend(observed.directories);
                }
                let report = run
                    .root
                    .join("reports/runs")
                    .join(&owner)
                    .join("report.json");
                if report.is_file() {
                    paths::reject_links(&run.root, &report)?;
                    let value = bounded_json(&report)?;
                    if sealed_owner(&value, &owner, qemu.as_deref())
                        && crate::layout::Layout::from_report(&value)?
                            == if retained {
                                crate::layout::Layout::RetainedDesktop
                            } else {
                                crate::layout::Layout::Legacy
                            }
                    {
                        completed.insert((owner, retained));
                    }
                }
            }
        }
    }
    let candidates: Vec<_> = candidate_directories
        .iter()
        .filter_map(|path| {
            let name = relative(&run.root, path).ok()?;
            let owner = cache_owner(&name)?;
            let retained = name.starts_with("reports/");
            (current_retired
                && owner == run.id
                && retained == (run.output_layout == crate::layout::Layout::RetainedDesktop)
                || completed.contains(&(owner.to_owned(), retained)))
            .then_some(path)
        })
        .collect();
    let mut references = Vec::new();
    if !candidates.is_empty() {
        retained_references(&run.root, &run.root.join("reports"), &mut references)?;
        retained_references(&run.root, &run.root.join("project"), &mut references)?;
        retained_references(&run.root, &run.root.join("specs"), &mut references)?;
    }
    collect_references(
        &json!({"checks":run.checks(),"commands":run.commands()}),
        &mut references,
    );
    // Keep generated latest-image/session/metadata references too.
    for entry in fs::read_dir(run.root.join("out")).into_iter().flatten() {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "json") {
            paths::reject_links(&run.root, &path)?;
            let value = bounded_json(&path)?;
            collect_references(&value, &mut references);
        }
    }
    let mut guest_active = if candidates.is_empty() {
        None
    } else {
        Some(quiescence()?)
    };
    let mut removed = Vec::new();
    if guest_active == Some(false) {
        for path in candidates {
            if !path.exists() {
                continue;
            }
            let name = relative(&run.root, path)?;
            let Some(owner) = cache_owner(&name) else {
                continue;
            };
            let retained = name.starts_with("reports/");
            if !(current_retired
                && owner == run.id
                && retained == (run.output_layout == crate::layout::Layout::RetainedDesktop)
                || completed.contains(&(owner.to_owned(), retained)))
                || referenced(&name, &references)
            {
                continue;
            }
            // Recheck the absolute, resolved target and every descendant immediately
            // before deletion. Only Cargo cache directories are admitted above.
            paths::reject_links(&run.root, path)?;
            let resolved = path.canonicalize().map_err(|e| e.to_string())?;
            let build_root = run
                .root
                .join(if name.starts_with("out/") {
                    "out/builds".to_owned()
                } else {
                    format!("reports/runs/{owner}/generated/builds")
                })
                .canonicalize()
                .map_err(|e| e.to_string())?;
            if !resolved.starts_with(&build_root) || resolved == build_root {
                return Err("cache cleanup escaped builds".into());
            }
            let mut subtree = Inventory::default();
            inventory(&run.root, &resolved, &mut subtree)?;
            if quiescence()? {
                guest_active = Some(true);
                break;
            }
            fs::remove_dir_all(&resolved).map_err(|e| format!("cache cleanup {name}: {e}"))?;
            removed.push(json!({"path":name,"logical_bytes":subtree.bytes,"files":subtree.files}));
        }
    }
    let after = inspect(&run.root)?;
    let receipt = json!({"schema_version":1,"record_kind":"output_cache_maintenance",
        "scope":"out only", "limit_bytes":LIMIT,"accounting":"logical file lengths",
        "cache_cleanup_scope":"completed Cargo leaves in out/builds and declared v2 generated/builds; final/failed evidence protected",
        "output_layout":run.output_layout.declaration(&run.id),
        "removed_retained_cache_bytes":removed.iter().filter(|v|v["path"].as_str().is_some_and(|p|p.starts_with("reports/"))).filter_map(|v|v["logical_bytes"].as_u64()).sum::<u64>(),
        "before_bytes":before.bytes,"after_bytes":after.bytes,"removed_cargo_caches":removed,
        "guest_active_cleanup_deferred":guest_active,"within_budget":after.bytes<=LIMIT,
        "known_handoff_policy":"exact hash-verified pinned QEMU + canonical receipt + actual QEMU/rustc quiescence; not guest success",
        "pinned_qemu_verified":qemu,
        "protected":"all out/session, acquisitions, final artifacts/maps, images, raw disks and execution evidence"});
    let name = if current_complete {
        "output-cache-after.json"
    } else {
        "output-cache-before.json"
    };
    write_json(&run.directory.join(name), &receipt)?;
    if after.bytes <= LIMIT {
        run.check(
            if current_complete {
                "output-cache-after"
            } else {
                "output-cache-before"
            },
            receipt.clone(),
        );
    }
    Ok(receipt)
}
pub fn admit(run: &mut Run) -> Result<(), String> {
    let receipt = maintain(run, false)?;
    let bytes = receipt["after_bytes"].as_u64().ok_or("cache size")?;
    let ceiling = admission_ceiling(run.output_layout);
    if bytes > ceiling {
        return Err(format!(
            "out budget: {bytes} bytes retained; admission ceiling {ceiling} for {:?}. Protected evidence was preserved; see output-cache-before.json",
            run.output_layout
        ));
    }
    Ok(())
}
/// One explicit fixed-size initializer, not permission for another build graph.
pub fn admit_owner_initialization(run: &mut Run) -> Result<(), String> {
    let receipt = maintain(run, false)?;
    let bytes = receipt["after_bytes"].as_u64().ok_or("cache size")?;
    let ceiling = LIMIT - 2_000_000_000 - OWNER_INITIALIZATION_RESERVE;
    if bytes > ceiling {
        return Err(format!(
            "out budget: {bytes} bytes retained; owner initialization ceiling {ceiling}; existing owner data was preserved"
        ));
    }
    run.check(
        "owner-volume-admission",
        json!({"out_bytes":bytes,
        "reserve_bytes":OWNER_INITIALIZATION_RESERVE,"live_writer_margin_bytes":2_000_000_000u64,
        "scope":"exact init-user-volume only; no build or guest execution"}),
    );
    Ok(())
}
fn admission_ceiling(layout: crate::layout::Layout) -> u64 {
    match layout {
        crate::layout::Layout::Legacy => LIMIT - HEADROOM,
        crate::layout::Layout::RetainedDesktop => LIMIT - 2_000_000_000 - DESKTOP_METADATA_RESERVE,
    }
}
pub fn within_limit(root: &Path) -> Result<(), String> {
    if inspect(root)?.bytes > LIMIT {
        Err("out exceeded its budget; controlled command must stop".into())
    } else {
        Ok(())
    }
}
// The orchestrator checks this conservative high-water mark while external
// writers run. Leave2GB for bounded writes between observations and cleanup.
// Every controlled process polls it once a second; concurrent guest workers
// share one inventory of at most that age instead of each walking out/.
pub fn writer_headroom(root: &Path) -> Result<(), String> {
    type Observation = Option<(PathBuf, std::time::Instant, Result<(), String>)>;
    static LATEST: std::sync::Mutex<Observation> = std::sync::Mutex::new(None);
    let mut latest = LATEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((observed_root, at, result)) = latest.as_ref()
        && observed_root == root
        && at.elapsed() < std::time::Duration::from_secs(1)
    {
        return result.clone();
    }
    let result = writer_headroom_now(root);
    *latest = Some((
        root.to_path_buf(),
        std::time::Instant::now(),
        result.clone(),
    ));
    result
}
fn writer_headroom_now(root: &Path) -> Result<(), String> {
    let mut observed = Inventory::default();
    inventory_mode(root, &root.join("out"), &mut observed, true)?;
    if observed.bytes > LIMIT {
        Err(format!(
            "out already exceeds its budget: {} bytes observed; controlled writer must stop",
            observed.bytes
        ))
    } else if observed.bytes > LIMIT - 2_000_000_000 {
        Err(format!(
            "out budget headroom exhausted: {} bytes observed; controlled writer must stop",
            observed.bytes
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_cache_cleanup_preserves_pins_unknown_owners_and_final_bytes() {
        let workspace = crate::test_support::Workspace::new();
        let owner = "host-123-456";
        let base = format!("reports/runs/{owner}/generated/builds/guest/pass-0/release");
        for leaf in ["deps", "build", ".fingerprint"] {
            fs::create_dir_all(workspace.root.join(format!("{base}/{leaf}"))).unwrap();
            fs::write(
                workspace.root.join(format!("{base}/{leaf}/intermediate")),
                b"cache",
            )
            .unwrap();
        }
        fs::write(
            workspace.root.join(format!("{base}/final.elf")),
            b"retained",
        )
        .unwrap();
        // Invalid compiler JSON is deliberately skipped: it is disposable, not
        // retained evidence. It must neither self-pin nor block cleanup.
        fs::write(
            workspace
                .root
                .join(format!("{base}/.fingerprint/compiler.json")),
            b"not JSON",
        )
        .unwrap();
        let record = json!({"record_kind":"host_run","id":owner,"success":false,
            "inputs_unchanged":true,"commands":[],
            "output_layout":crate::layout::Layout::RetainedDesktop.declaration(owner),
            "pinned":format!("C:/WORKSPACE/{base}/deps/intermediate").replace('/',"\\")});
        write_json(
            &workspace
                .root
                .join(format!("reports/runs/{owner}/report.json")),
            &record,
        )
        .unwrap();
        let foreign = "reports/runs/host-7-8/generated/builds/pass-0/release/deps";
        fs::create_dir_all(workspace.root.join(foreign)).unwrap();
        fs::write(
            workspace.root.join(format!("{foreign}/intermediate")),
            b"unknown",
        )
        .unwrap();
        let mut run = Run::new(workspace.root.clone()).unwrap();
        let receipt = maintain_with(&mut run, false, || Ok(false)).unwrap();
        assert_eq!(receipt["before_bytes"], receipt["after_bytes"]);
        assert_eq!(receipt["removed_cargo_caches"].as_array().unwrap().len(), 2);
        assert!(receipt["removed_retained_cache_bytes"].as_u64().unwrap() > 0);
        assert!(
            workspace
                .root
                .join(format!("{base}/deps/intermediate"))
                .is_file()
        );
        assert!(workspace.root.join(format!("{base}/final.elf")).is_file());
        assert!(
            workspace
                .root
                .join(format!("{foreign}/intermediate"))
                .is_file()
        );
        assert_eq!(
            fs::read(
                workspace
                    .root
                    .join(format!("reports/runs/{owner}/report.json"))
            )
            .unwrap(),
            serde_json::to_vec_pretty(&record)
                .unwrap()
                .into_iter()
                .chain([b'\n'])
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn layout_specific_admission_preserves_legacy_reserve_and_live_guard() {
        assert_eq!(
            admission_ceiling(crate::layout::Layout::Legacy),
            24_000_000_000
        );
        assert_eq!(
            admission_ceiling(crate::layout::Layout::RetainedDesktop),
            29_865_782_272
        );
        assert!(admission_ceiling(crate::layout::Layout::RetainedDesktop) < 30_000_000_000);
        assert_eq!(OWNER_INITIALIZATION_RESERVE, 320 * 1024 * 1024);
        assert!(OWNER_INITIALIZATION_RESERVE > 2 * crate::storage::DISK_BYTES as u64);
    }
    #[test]
    fn sealed_and_current_completion_cannot_cross_layouts_for_the_same_owner() {
        use crate::layout::Layout;
        for current in [false, true] {
            for layout in [Layout::Legacy, Layout::RetainedDesktop] {
                let workspace = crate::test_support::Workspace::new();
                let mut run = Run::new(workspace.root.clone()).unwrap();
                run.output_layout = layout;
                let owner = if current {
                    run.id.clone()
                } else {
                    "host-123-456".to_owned()
                };
                let legacy = format!("out/builds/{owner}/pass-0/release/deps");
                let retained = Layout::RetainedDesktop.path(&owner, &legacy).unwrap();
                for name in [&legacy, &retained] {
                    fs::create_dir_all(workspace.root.join(name)).unwrap();
                    fs::write(workspace.root.join(name).join("intermediate"), b"cache").unwrap();
                }
                if !current {
                    write_json(&workspace.root.join(format!("reports/runs/{owner}/report.json")),
                        &json!({"record_kind":"host_run","id":owner,"success":true,"inputs_unchanged":true,
                            "commands":[],"output_layout":layout.declaration(&owner)})).unwrap();
                }
                let receipt = maintain_with(&mut run, current, || Ok(false)).unwrap();
                assert_eq!(receipt["removed_cargo_caches"].as_array().unwrap().len(), 1);
                assert_eq!(
                    workspace.root.join(&legacy).exists(),
                    layout != Layout::Legacy
                );
                assert_eq!(
                    workspace.root.join(&retained).exists(),
                    layout != Layout::RetainedDesktop
                );
            }
        }
    }
    #[test]
    fn cleanup_allowlist_never_admits_owner_disks_or_final_artifacts() {
        for name in [
            "out/session/user.raw",
            "out/builds/host-123-456/pass-0/x86_64-unknown-none/release/door-storage",
            "out/qemu/host-123-456/release/deps",
            "out/builds/unknown/pass-0/release/deps",
            "out/acquisitions/host-123-456/release/deps",
        ] {
            assert!(cache_owner(name).is_none(), "{name}");
        }
        assert_eq!(
            cache_owner("out/builds/host-123-456/pass-0/x86_64-unknown-none/release/deps"),
            Some("host-123-456")
        );
    }
    #[test]
    fn nested_artifact_map_absolute_and_path_arrays_veto_pruning() {
        let mut references = Vec::new();
        collect_references(
            &json!({"artifacts":[{"link_map":{"path":"out/builds/host-1/pass-0/release/deps/a.map"}}],
            "paths":["C:\\workspace\\out\\builds\\host-1\\pass-1\\release\\deps\\a"]}),
            &mut references,
        );
        assert!(referenced(
            "out/builds/host-1/pass-0/release/deps",
            &references
        ));
        assert!(referenced(
            "out/builds/host-1/pass-1/release/deps",
            &references
        ));
        assert!(!referenced(
            "out/builds/host-1/pass-0/release/build",
            &references
        ));
        assert!(!referenced(
            "out/builds/host-1/pass-0/release/dep",
            &references
        ));
    }
    #[test]
    fn logical_accounting_includes_entire_out_but_excludes_other_roots() {
        let workspace = crate::test_support::Workspace::new();
        for name in ["out/session", "out/builds", "target", "reports/runs"] {
            fs::create_dir_all(workspace.root.join(name)).unwrap();
        }
        fs::write(workspace.root.join("out/session/user.raw"), [1; 29]).unwrap();
        fs::write(workspace.root.join("out/builds/map"), [2; 7]).unwrap();
        fs::write(workspace.root.join("target/cache"), [3; 100]).unwrap();
        fs::write(workspace.root.join("reports/runs/evidence"), [4; 100]).unwrap();
        assert_eq!(status(&workspace.root).unwrap()["bytes"], 36);
    }
    #[cfg(windows)]
    #[test]
    fn exclusive_lease_has_no_stale_pid_ownership() {
        let workspace = crate::test_support::Workspace::new();
        let first = Lease::acquire(&workspace.root).unwrap();
        assert!(Lease::acquire(&workspace.root).is_err());
        drop(first);
        assert!(Lease::acquire(&workspace.root).is_ok());
    }
    fn fixture() -> (crate::test_support::Workspace, Run) {
        let workspace = crate::test_support::Workspace::new();
        for directory in [
            "out/session",
            "out/acquisitions",
            "out/builds/host-1-2/pass-0/release/deps",
            "out/builds/host-1-2/pass-0/release/build",
            "out/builds/host-3-4/pass-0/release/deps",
            "reports/runs/host-1-2",
        ] {
            fs::create_dir_all(workspace.root.join(directory)).unwrap();
        }
        for file in [
            "out/session/user.raw",
            "out/acquisitions/code.fd",
            "out/builds/host-1-2/pass-0/release/deps/artifact",
            "out/builds/host-1-2/pass-0/release/build/intermediate",
            "out/builds/host-1-2/pass-0/release/final.elf",
            "out/builds/host-3-4/pass-0/release/deps/intermediate",
        ] {
            fs::write(workspace.root.join(file), b"preserve identity").unwrap();
        }
        write_json(
            &workspace.root.join("reports/runs/host-1-2/report.json"),
            &json!({"record_kind":"host_run","id":"host-1-2","success":true,"inputs_unchanged":true,
                "commands":[],"path":"OUT/BUILDS/host-1-2/pass-0/release/deps/artifact"}),
        )
        .unwrap();
        let run = Run::new(workspace.root.clone()).unwrap();
        (workspace, run)
    }
    #[test]
    fn actual_cleanup_preserves_references_live_builds_and_owner_bytes() {
        let (workspace, mut run) = fixture();
        let receipt = maintain_with(&mut run, false, || Ok(false)).unwrap();
        assert_eq!(receipt["removed_cargo_caches"].as_array().unwrap().len(), 1);
        assert!(
            !workspace
                .root
                .join("out/builds/host-1-2/pass-0/release/build")
                .exists()
        );
        for file in [
            "out/session/user.raw",
            "out/acquisitions/code.fd",
            "out/builds/host-1-2/pass-0/release/deps/artifact",
            "out/builds/host-1-2/pass-0/release/final.elf",
            "out/builds/host-3-4/pass-0/release/deps/intermediate",
        ] {
            assert_eq!(
                fs::read(workspace.root.join(file)).unwrap(),
                b"preserve identity"
            );
        }
        let again = maintain_with(&mut run, false, || Ok(false)).unwrap();
        assert!(again["removed_cargo_caches"].as_array().unwrap().is_empty());
    }
    #[test]
    fn active_guest_or_unknown_evidence_vetoes_deletion() {
        let (workspace, mut run) = fixture();
        let receipt = maintain_with(&mut run, false, || Ok(true)).unwrap();
        assert!(
            receipt["removed_cargo_caches"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        fs::write(
            workspace.root.join("reports/runs/host-1-2/invalid.json"),
            b"broken",
        )
        .unwrap();
        assert!(maintain_with(&mut run, false, || Ok(false)).is_err());
        assert!(
            workspace
                .root
                .join("out/builds/host-1-2/pass-0/release/build/intermediate")
                .is_file()
        );
    }
    #[test]
    fn uncertain_process_cleanup_cannot_authorize_cache_deletion() {
        assert!(!retired(&[json!({"cleanup_error":"tree kill failed"})]));
        assert!(!retired(&[
            json!({"cleanup_error":null,"lifecycle":"running"})
        ]));
        assert!(retired(&[
            json!({"cleanup_error":null,"lifecycle":"completed","outcome":"passed","exit_code":0})
        ]));
        let (workspace, mut run) = fixture();
        write_json(
            &workspace.root.join("reports/runs/host-1-2/report.json"),
            &json!({"commands":[{"cleanup_error":"tree kill failed"}]}),
        )
        .unwrap();
        let receipt = maintain_with(&mut run, false, || panic!("no completed candidates")).unwrap();
        assert!(
            receipt["removed_cargo_caches"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            workspace
                .root
                .join("out/builds/host-1-2/pass-0/release/build/intermediate")
                .is_file()
        );
    }
    #[test]
    fn unknown_or_mismatched_ownership_is_never_a_completed_build() {
        for name in [
            "host-other",
            "host-1",
            "host-01-2",
            "host-1-0",
            "host-1-2-extra",
            "host-1-4294967296",
        ] {
            assert!(!canonical_owner(name), "{name}");
        }
        assert!(canonical_owner("host-1792553435241221300-12020"));
        let sealed = json!({"record_kind":"host_run","id":"host-1-2","commands":[],"success":false,"inputs_unchanged":true});
        assert!(sealed_owner(&sealed, "host-1-2", None));
        assert!(!sealed_owner(&sealed, "host-3-4", None));
        assert!(!sealed_owner(&json!({}), "host-1-2", None));
        let (workspace, mut run) = fixture();
        write_json(
            &workspace.root.join("reports/runs/host-1-2/report.json"),
            &json!({}),
        )
        .unwrap();
        let receipt = maintain_with(&mut run, false, || panic!("unknown owner")).unwrap();
        assert!(
            receipt["removed_cargo_caches"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            workspace
                .root
                .join("out/builds/host-1-2/pass-0/release/build/intermediate")
                .is_file()
        );
    }
    fn handoff(qemu: &str) -> Value {
        json!({"argv":[qemu,"-display","none"],"pid":123,"lifecycle":"running","outcome":"running",
            "required":false,"exit_code":null,"expected_exit_code":0,"timed_out":false,"output_limit_exceeded":false,
            "spawn_error":null,"controller_error":null,"cleanup_error":null,"output_budget_error":null})
    }
    #[test]
    fn handoff_exception_requires_exact_pinned_executable_and_complete_clean_receipt() {
        let qemu = "C:/Program Files/qemu/qemu-system-x86_64.exe";
        let good = handoff(qemu);
        assert!(eligible_commands(std::slice::from_ref(&good), Some(qemu)));
        assert!(!eligible_commands(std::slice::from_ref(&good), None));
        for path in [
            "D:/other/qemu-system-x86_64.exe",
            "C:/Program Files/qemu/qemu-system-x86_64.exe.old",
            "C:/Program Files/qemu/other-qemu-system-x86_64.exe",
        ] {
            let mut bad = good.clone();
            bad["argv"] = json!([path]);
            assert!(!eligible_commands(&[bad], Some(qemu)), "{path}");
        }
        for (key, value) in [
            ("pid", json!(0)),
            ("pid", json!(4294967296u64)),
            ("pid", Value::Null),
            ("outcome", json!("failed")),
            ("lifecycle", json!("unknown")),
            ("required", json!(true)),
            ("exit_code", json!(0)),
            ("spawn_error", json!("failed")),
            ("controller_error", json!("failed")),
            ("cleanup_error", json!("failed")),
            ("output_budget_error", json!("failed")),
            ("timed_out", json!(true)),
            ("output_limit_exceeded", json!(true)),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(!eligible_commands(&[bad], Some(qemu)), "{key}");
        }
        for key in [
            "pid",
            "cleanup_error",
            "spawn_error",
            "output_budget_error",
            "controller_error",
        ] {
            let mut bad = good.clone();
            bad.as_object_mut().unwrap().remove(key);
            assert!(!eligible_commands(&[bad], Some(qemu)), "missing{key}");
        }
        for lifecycle in [None, Some(Value::Null), Some(json!("completed"))] {
            for path in [qemu, "C:/unknown/background-writer.exe"] {
                let mut bad = good.clone();
                bad["argv"] = json!([path]);
                if let Some(lifecycle) = lifecycle.clone() {
                    bad["lifecycle"] = lifecycle;
                } else {
                    bad.as_object_mut().unwrap().remove("lifecycle");
                }
                assert!(
                    !eligible_commands(&[bad], Some(qemu)),
                    "stripped handoff: {path}"
                );
            }
        }
    }
    #[test]
    fn historical_handoff_still_requires_actual_quiescence_and_unchanged_emulator() {
        let (workspace, mut run) = fixture();
        fs::create_dir_all(workspace.root.join("host-tools")).unwrap();
        fs::create_dir_all(workspace.root.join("profiles/hardware")).unwrap();
        let executable = workspace.root.join("host-tools/qemu-system-x86_64.exe");
        fs::write(&executable, b"test-only pinned identity; never executed").unwrap();
        let name = executable.to_str().unwrap();
        write_json(&workspace.root.join("profiles/hardware/qemu-diagnostic-x64.json"),
            &json!({"qemu":{"path":name,"sha256":crate::evidence::file_hash(&executable).unwrap()}})).unwrap();
        write_json(&workspace.root.join("reports/runs/host-1-2/report.json"),
            &json!({"record_kind":"host_run","id":"host-1-2","success":false,"inputs_unchanged":true,
                "commands":[handoff(name)],"path":"out/builds/host-1-2/pass-0/release/deps/artifact"})).unwrap();
        let queries = std::cell::Cell::new(0);
        let receipt = maintain_with(&mut run, false, || {
            queries.set(queries.get() + 1);
            Ok(queries.get() > 1)
        })
        .unwrap();
        assert!(
            receipt["removed_cargo_caches"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(receipt["guest_active_cleanup_deferred"], true);
        let receipt = maintain_with(&mut run, false, || Ok(false)).unwrap();
        assert_eq!(receipt["removed_cargo_caches"].as_array().unwrap().len(), 1);
        fs::write(&executable, b"changed identity").unwrap();
        assert!(verified_qemu(&workspace.root).is_none());
        assert!(workspace.root.join("out/session/user.raw").is_file());
        assert!(
            workspace
                .root
                .join("out/builds/host-1-2/pass-0/release/final.elf")
                .is_file()
        );
    }
    #[test]
    fn oversized_completion_or_metadata_is_rejected_before_parsing() {
        let workspace = crate::test_support::Workspace::new();
        let path = workspace.root.join("oversized.json");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_JSON + 1).unwrap();
        drop(file);
        assert_eq!(
            bounded_json(&path),
            Err("oversized retained JSON prevents safe cache cleanup".into())
        );
    }
}
