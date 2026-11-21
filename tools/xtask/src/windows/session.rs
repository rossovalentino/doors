//! The Windows process borrows a private copy of already verified native storage.
use crate::{
    evidence::{Run, file_hash, relative},
    paths, services,
};
use serde_json::{Value, json};
use std::{fs, path::Path};
pub fn clone_saved(run: &Run, saved: &str) -> Result<String, String> {
    let source = paths::existing(&run.root, saved)?;
    if services::inspect(&source)?["catalog"]["state"] != "saved" {
        return Err("Windows service graph requires saved native state".into());
    }
    let directory = paths::fresh_directory(&run.root, &format!("out/storage/{}-p09", run.id))?;
    for name in ["initial.raw", "data.raw"] {
        fs::copy(&source, directory.join(name)).map_err(|e| e.to_string())?;
    }
    let path = relative(&run.root, &directory.join("data.raw"))?;
    println!("P09 private persisted disk: {path}");
    Ok(path)
}
pub fn evidence(root: &Path, id: &str) -> Result<Value, String> {
    let mut record = json!({"schema_version":1,"record_kind":"windows_service_disk","root_run":id});
    for (key, name) in [("initial", "initial.raw"), ("final", "data.raw")] {
        let path = format!("out/storage/{id}-p09/{name}");
        let file = paths::existing(root, &path)?;
        record[key] = json!({"path":path,"sha256":file_hash(&file)?});
    }
    Ok(record)
}
pub fn validate(root: &Path, id: &str, suite: &Value, guests: &[Value]) -> Result<(), String> {
    let record = evidence(root, id)?;
    if suite["windows"] != record
        || guests.len() != 1
        || record["initial"]["sha256"] != suite["services"]["saved"]["sha256"]
        || record["final"]["sha256"] != record["initial"]["sha256"]
    {
        return Err(
            "Windows private disk changed or was not cloned from saved native state".into(),
        );
    }
    let guest = &guests[0];
    let token = guest["run_token"].as_str().ok_or("Windows guest token")?;
    let snapshot = format!("out/qemu/{token}/data-after.raw");
    let before = services::inspect(&paths::existing(
        root,
        record["initial"]["path"].as_str().unwrap(),
    )?)?;
    let after = services::inspect(&paths::existing(root, &snapshot)?)?;
    if guest["scenario"] != super::MODE
        || guest["data_disk"]["path"] != record["final"]["path"]
        || guest["data_disk"]["before"] != before
        || guest["data_disk"]["after"]["path"] != snapshot
        || guest["data_disk"]["after"]["audit"] != after
        || guest["data_disk"]["after"]["sha256"] != record["initial"]["sha256"]
        || before != after
        || !guest["data_disk"]["audit_error"].is_null()
    {
        return Err("Windows before/after persistent audit differs".into());
    }
    Ok(())
}
