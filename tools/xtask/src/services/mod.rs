//! P08 service process fixtures and independent persistent-workload evidence.
mod audit;
pub mod checked_remove;
pub mod checked_replace;
pub mod demo;
pub mod reconnect;
pub mod scenarios;
pub mod session;
pub mod verify;
use crate::{
    evidence::{Run, relative, write_json},
    paths,
};
pub use audit::inspect;
pub use audit::inspect_windows;
use serde_json::json;
use std::fs;

pub const PROFILE: &str = "profiles/hardware/qemu-services-x64.json";
pub fn is_mode(mode: &str) -> bool {
    matches!(
        mode,
        "services-save" | "services-reopen" | "services-restart" | "services-failure"
    )
}
pub fn inspect_mode(path: &std::path::Path, mode: &str) -> Result<serde_json::Value, String> {
    audit::inspect_mode(path, mode == "services-failure")
}

pub fn create(run: &Run) -> Result<String, String> {
    let directory = paths::fresh_directory(&run.root, &format!("out/storage/{}-p08", run.id))?;
    let bytes = crate::persistence::format::image(false);
    let record = audit::inspect_bytes(&bytes)?;
    for name in ["initial.raw", "data.raw"] {
        fs::write(directory.join(name), &bytes).map_err(|error| error.to_string())?;
    }
    write_json(&directory.join("initial-audit.json"), &record)?;
    let path = relative(&run.root, &directory.join("data.raw"))?;
    println!("P08 service fixture: {path}");
    Ok(path)
}

pub fn validate_sources(report: &serde_json::Value) -> Result<(), String> {
    let files = report["source_snapshot"]["files"]
        .as_array()
        .ok_or("P08 source identity")?;
    for path in [
        "tools/xtask/src/services/mod.rs",
        "tools/xtask/src/services/audit.rs",
        "tools/xtask/src/services/session.rs",
        "tools/xtask/src/services/scenarios.rs",
        "tools/xtask/src/services/demo.rs",
        "tools/xtask/src/services/verify.rs",
        PROFILE,
        "profiles/build/initfs-p08.json",
        "specs/abi/native-platform-v1.md",
        "services/Cargo.toml",
        "services/linker.ld",
        "tests/fixtures/native-workload/Cargo.toml",
        "tests/fixtures/native-workload/linker.ld",
    ] {
        let matching: Vec<_> = files.iter().filter(|file| file["path"] == path).collect();
        if matching.len() != 1
            || matching[0]["sha256"]
                .as_str()
                .is_none_or(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(format!("missing/duplicate P08 source identity: {path}"));
        }
    }
    Ok(())
}

pub fn record_audit(run: &mut Run, path: &str) -> Result<(), String> {
    let checked = inspect(&paths::existing(&run.root, path)?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&checked).map_err(|e| e.to_string())?
    );
    run.check("p08-services-audit", json!(checked));
    Ok(())
}
