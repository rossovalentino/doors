//! Retained save/reopen/restart/corrupt-refusal chain for the service platform.
use super::{PROFILE, audit, inspect_mode};
use crate::{
    evidence::{Run, file_hash, relative},
    paths,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Session {
    root: PathBuf,
    id: String,
    current: String,
    baseline: Option<PathBuf>,
    guests: Vec<Value>,
}
impl Session {
    pub fn new(run: &Run) -> Result<Self, String> {
        Ok(Self {
            root: run.root.clone(),
            id: run.id.clone(),
            current: super::create(run)?,
            baseline: None,
            guests: Vec::new(),
        })
    }
    pub fn disk(&mut self, mode: &str) -> Result<&str, String> {
        if mode == "services-failure" {
            let baseline = self.baseline.as_ref().ok_or("P08 saved baseline missing")?;
            let mut bytes = fs::read(baseline).map_err(|e| e.to_string())?;
            let at = audit::corruption_offset(&bytes)?;
            bytes[at] ^= 1;
            let directory = paths::fresh_directory(
                &self.root,
                &format!("out/storage/{}-p08-corrupt", self.id),
            )?;
            for name in ["initial.raw", "data.raw"] {
                fs::write(directory.join(name), &bytes).map_err(|e| e.to_string())?;
            }
            self.current = relative(&self.root, &directory.join("data.raw"))?;
        }
        Ok(&self.current)
    }
    pub fn observe(&mut self, guest: Value) -> Result<(), String> {
        if guest["scenario"] == "services-save" {
            self.baseline = Some(paths::existing(
                &self.root,
                guest["data_disk"]["after"]["path"]
                    .as_str()
                    .ok_or("P08 saved snapshot")?,
            )?);
        }
        self.guests.push(guest);
        Ok(())
    }
    pub fn evidence(&self) -> Result<Value, String> {
        evidence(&self.root, &self.id, &self.guests)
    }
}
fn binding(root: &Path, path: &str) -> Result<Value, String> {
    Ok(json!({"path":path,"sha256":file_hash(&paths::existing(root, path)?)?}))
}
fn evidence(root: &Path, id: &str, guests: &[Value]) -> Result<Value, String> {
    if guests.len() != 4 {
        return Err("P08 requires all four service process scenarios".into());
    }
    Ok(
        json!({"schema_version":1,"record_kind":"service_session","root_run":id,"profile":PROFILE,
        "initial":binding(root,&format!("out/storage/{id}-p08/initial.raw"))?,
        "saved":guests[0]["data_disk"]["after"],
        "final":binding(root,&format!("out/storage/{id}-p08/data.raw"))?,
        "corrupt_initial":binding(root,&format!("out/storage/{id}-p08-corrupt/initial.raw"))?,
        "corrupt_final":binding(root,&format!("out/storage/{id}-p08-corrupt/data.raw"))?}),
    )
}
pub fn validate(root: &Path, id: &str, suite: &Value, guests: &[Value]) -> Result<(), String> {
    if evidence(root, id, guests)? != suite["services"] {
        return Err("P08 service session binding differs".into());
    }
    let initial_path = paths::existing(root, &format!("out/storage/{id}-p08/initial.raw"))?;
    let initial = fs::read(initial_path).map_err(|e| e.to_string())?;
    if initial != crate::persistence::format::image(false) {
        return Err("P08 original disk fixture changed".into());
    }
    let mut prior = audit::inspect_bytes(&initial)?;
    let mut saved = Vec::new();
    for (index, mode) in [
        "services-save",
        "services-reopen",
        "services-restart",
        "services-failure",
    ]
    .into_iter()
    .enumerate()
    {
        let guest = &guests[index];
        let suffix = if index == 3 { "p08-corrupt" } else { "p08" };
        let disk = format!("out/storage/{id}-{suffix}/data.raw");
        let data = &guest["data_disk"];
        if index == 3 {
            let corrupted_path =
                paths::existing(root, &format!("out/storage/{id}-p08-corrupt/initial.raw"))?;
            let corrupted = fs::read(&corrupted_path).map_err(|e| e.to_string())?;
            let mut expected = saved.clone();
            let at = audit::corruption_offset(&expected)?;
            expected[at] ^= 1;
            if corrupted != expected {
                return Err("P08 corruption is not the sole declared catalog CRC bit".into());
            }
            prior = inspect_mode(&corrupted_path, mode)?;
        }
        let token = guest["run_token"].as_str().ok_or("P08 guest token")?;
        let snapshot_name = format!("out/qemu/{token}/data-after.raw");
        if guest["scenario"] != mode
            || data["path"] != disk
            || data["before"] != prior
            || data["after"]["path"] != snapshot_name
            || !data["audit_error"].is_null()
        {
            return Err("P08 before/after scenario chain differs".into());
        }
        let snapshot = paths::existing(root, &snapshot_name)?;
        let inspected = inspect_mode(&snapshot, mode)?;
        if inspected != data["after"]["audit"] || inspected["sha256"] != data["after"]["sha256"] {
            return Err("P08 retained disk audit changed".into());
        }
        let bytes = fs::read(snapshot).map_err(|e| e.to_string())?;
        if index == 0 {
            if inspected["catalog"]["state"] != "saved" {
                return Err("P08 save did not persist workload".into());
            }
            let mutable = |at: usize| {
                (133120 * 512..198656 * 512).contains(&at)
                    || (202752 * 512..206848 * 512).contains(&at)
            };
            if initial
                .iter()
                .zip(&bytes)
                .enumerate()
                .any(|(at, (a, b))| a != b && !mutable(at))
            {
                return Err("P08 save changed unrelated disk regions".into());
            }
            saved = bytes;
        } else if inspected["sha256"] != prior["sha256"] {
            return Err("P08 reopen/restart/refusal mutated durable bytes".into());
        }
        prior = inspected;
    }
    for (suffix, last) in [("p08", 2), ("p08-corrupt", 3)] {
        if file_hash(&paths::existing(
            root,
            &format!("out/storage/{id}-{suffix}/data.raw"),
        )?)? != guests[last]["data_disk"]["after"]["sha256"]
        {
            return Err("P08 final writable disk differs from retained last boot".into());
        }
    }
    Ok(())
}
