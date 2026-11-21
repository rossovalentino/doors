//! `cargo host-assets`: the Microsoft-owned fonts, icons, cursors, sounds
//! and wallpapers, every generated asset and the IME dictionaries are not in
//! the repository. Several workspace crates embed them, so this first writes
//! a zero-filled placeholder for each missing file (letting the workspace
//! compile), then runs `cargo xtask host-assets acquire` (copy the sources
//! from this Windows PC) and `cargo xtask host-assets export` (regenerate
//! the rest; see tools/xtask/src/assets/host.rs).
use std::{fs, path::Path, process::Command, process::ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("host-assets: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let inventory: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("assets/host-assets.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let _ = fs::remove_file(root.join("assets/.host-assets-ok"));
    // Every listed file, with its size: the sources, and the generated files
    // as `"folder": "name:bytes name:bytes"`.
    let mut files = Vec::new();
    for entry in inventory["sources"].as_array().ok_or("inventory")? {
        let path = entry["path"].as_str().ok_or("inventory path")?;
        files.push((path.to_string(), entry["bytes"].as_u64().ok_or("inventory size")?));
    }
    for (folder, names) in inventory["generated"].as_object().ok_or("inventory")? {
        for item in names.as_str().ok_or("inventory")?.split(' ') {
            let (name, bytes) = item.rsplit_once(':').ok_or("inventory size")?;
            let bytes = bytes.parse().map_err(|_| "inventory size")?;
            files.push((format!("{folder}/{name}"), bytes));
        }
    }
    let mut placeholders = 0;
    for (path, bytes) in files {
        if !path.starts_with("assets/") || path.contains("..") {
            return Err(format!("inventory path {path}"));
        }
        let target = root.join(&path);
        if target.exists() {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&target, vec![0u8; bytes as usize]).map_err(|e| format!("{path}: {e}"))?;
        placeholders += 1;
    }
    println!("{placeholders} placeholders written");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    for step in ["acquire", "export"] {
        let status = Command::new(&cargo)
            .args(["xtask", "host-assets", step])
            .current_dir(&root)
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("`cargo xtask host-assets {step}` failed"));
        }
    }
    Ok(())
}
