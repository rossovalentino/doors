//! Private baseline cloning and retained-image chain for the P07 guest schedule.
use super::{
    HOME_FIRST, HOME_SECTORS, LOG_FIRST, LOG_SECTORS, PROFILE, format, inspect, inspect_bytes,
    require,
};
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
    pub guests: Vec<Value>,
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
    pub fn disk(&mut self, run: &Run, mode: &str) -> Result<&str, String> {
        if super::interruption_stage(mode).is_some() {
            let baseline = self
                .baseline
                .as_ref()
                .ok_or("saved persistence baseline missing")?;
            let directory =
                paths::fresh_directory(&self.root, &format!("out/storage/{}-{mode}", self.id))?;
            for file in ["initial.raw", "data.raw"] {
                fs::copy(baseline, directory.join(file)).map_err(|e| e.to_string())?;
            }
            self.current = relative(&self.root, &directory.join("data.raw"))?;
        } else if mode == "persist-full" {
            self.current = super::create_size(run, true)?;
        }
        Ok(&self.current)
    }
    pub fn observe(&mut self, guest: Value) -> Result<(), String> {
        if guest["scenario"] == "persist-save" {
            self.baseline = Some(paths::existing(
                &self.root,
                guest["data_disk"]["after"]["path"]
                    .as_str()
                    .ok_or("saved snapshot")?,
            )?);
        }
        self.guests.push(guest);
        Ok(())
    }
    pub fn evidence(&self) -> Result<Value, String> {
        evidence(&self.root, &self.id, &self.guests)
    }
}
pub fn evidence(root: &Path, id: &str, guests: &[Value]) -> Result<Value, String> {
    require(
        guests.len() == 18,
        "P07 requires all18 persistence processes",
    )?;
    let initial = format!("out/storage/{id}-p07/initial.raw");
    let tiny = format!("out/storage/{id}-full/initial.raw");
    let binding = |name: &str| -> Result<Value, String> {
        Ok(json!({"path":name,"sha256":file_hash(&paths::existing(root,name)?)?}))
    };
    let branches = guests
        .iter()
        .filter(|g| super::interruption_stage(g["scenario"].as_str().unwrap_or("")).is_some())
        .map(|g| {
            let mode = g["scenario"].as_str().ok_or("branch mode")?;
            let first = format!("out/storage/{id}-{mode}/initial.raw");
            let last = format!("out/storage/{id}-{mode}/data.raw");
            Ok(json!({"mode":mode,"initial":binding(&first)?,"final":binding(&last)?}))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(
        json!({"schema_version":1,"record_kind":"persistence_session","root_run":id,"profile":PROFILE,"initial":binding(&initial)?,"baseline":guests[0]["data_disk"]["after"],"branches":branches,"capacity_initial":binding(&tiny)?}),
    )
}
pub fn validate(root: &Path, id: &str, suite: &Value, guests: &[Value]) -> Result<(), String> {
    require(
        evidence(root, id, guests)? == suite["persistence"],
        "P07 session bindings differ",
    )?;
    for (suffix, tiny) in [("p07", false), ("full", true)] {
        let path = paths::existing(root, &format!("out/storage/{id}-{suffix}/initial.raw"))?;
        require(
            inspect(&path)? == inspect_bytes(&format::image(tiny))?,
            "P07 original fixture differs",
        )?;
    }
    let mut path = format!("out/storage/{id}-p07/data.raw");
    let mut previous = inspect(&paths::existing(
        root,
        &format!("out/storage/{id}-p07/initial.raw"),
    )?)?;
    let mut baseline = Value::Null;
    for (index, (&(scenario, mode), guest)) in
        super::scenarios::SCHEDULE.iter().zip(guests).enumerate()
    {
        let branch = super::interruption_stage(mode).is_some();
        if branch {
            path = format!("out/storage/{id}-{mode}/data.raw");
            previous = baseline.clone();
            let cloned = paths::existing(root, &format!("out/storage/{id}-{mode}/initial.raw"))?;
            require(
                inspect(&cloned)? == previous,
                "P07 crash branch did not clone saved baseline",
            )?;
        } else if mode == "persist-full" {
            path = format!("out/storage/{id}-full/data.raw");
            previous = inspect(&paths::existing(
                root,
                &format!("out/storage/{id}-full/initial.raw"),
            )?)?;
        }
        let data = &guest["data_disk"];
        require(
            guest["scenario"] == mode
                && data["path"] == path
                && data["before"] == previous
                && data["audit_error"].is_null(),
            "P07 mode/image/hash chain differs",
        )?;
        let snapshot = format!(
            "out/qemu/{}/data-after.raw",
            guest["run_token"].as_str().ok_or("P07 token")?
        );
        require(
            data["after"]["path"] == snapshot,
            "P07 snapshot must belong to its actual process",
        )?;
        let actual = inspect(&paths::existing(root, &snapshot)?)?;
        require(
            data["after"]["audit"] == actual && data["after"]["sha256"] == actual["sha256"],
            "P07 raw snapshot audit differs",
        )?;
        let expected_state = match mode {
            "persist-full" => "capacity",
            "persist-save" | "persist-reopen" | "recover-old" | "persist-old"
            | "journal-prepare" => "old",
            _ => "new",
        };
        require(
            actual["catalog"]["state"] == expected_state,
            "P07 durable catalog state differs from checkpoint expectation",
        )?;
        if expected_state == "new" {
            require(
                actual["catalog"]["associations"] == baseline["catalog"]["associations"],
                "P07 rename changed durable file identity/ownership",
            )?;
        }
        if mode == "persist-full" {
            require(
                actual["ext2"]["free_blocks"] == 0 && actual["catalog"]["capacity_files"] == 9,
                "P07 physical capacity fixture is not full",
            )?;
        }
        let serial = fs::read_to_string(paths::existing(
            root,
            guest["serial"]["path"].as_str().ok_or("P07 serial")?,
        )?)
        .map_err(|e| e.to_string())?;
        if mode != "persist-full" {
            let generation = if branch {
                2
            } else {
                previous["journal"]["generation"]
                    .as_u64()
                    .ok_or("prior generation")?
            };
            let prefix = if branch {
                "DOOR_P07 INTERRUPT "
            } else {
                "DOOR_P07 RECOVERY "
            };
            let line = serial
                .lines()
                .find(|l| l.starts_with(prefix))
                .ok_or("P07 generation marker")?;
            require(
                line.ends_with(&format!(" generation={generation}")),
                "P07 serial generation differs from actual journal",
            )?;
        }
        if index == 0 {
            baseline = actual.clone();
        }
        if matches!(mode, "persist-reopen" | "persist-old" | "persist-new") {
            require(
                actual == previous,
                "P07 idempotent reopen changed raw bytes",
            )?;
        }
        if mode.starts_with("recover-") {
            require(
                actual["effective_home_sha256"] == previous["effective_home_sha256"]
                    && !actual["journal"]["committed"].as_bool().unwrap_or(true),
                "P07 recovery differs from independent replay",
            )?;
        }
        if branch {
            let stage = super::interruption_stage(mode).unwrap();
            let generation = baseline["journal"]["generation"]
                .as_u64()
                .ok_or("baseline journal generation")?;
            let expected_gen = if stage == "prepare" {
                generation
            } else {
                generation + 1
            };
            require(
                actual["journal"]["generation"] == expected_gen
                    && actual["journal"]["committed"]
                        == matches!(stage, "commit" | "first_home" | "home"),
                "P07 interruption header does not represent requested commit point",
            )?;
            if stage == "prepare" {
                require(
                    actual["effective_home_sha256"] == baseline["effective_home_sha256"],
                    "P07 prepare modified home",
                )?;
            }
        }
        if scenario.ends_with("-reopen") || mode == "persist-full" {
            require(
                file_hash(&crate::storage::confined(root, &path)?)?
                    == actual["sha256"].as_str().ok_or("final image hash")?,
                "P07 final writable image differs from last snapshot",
            )?;
        }
        previous = actual;
    }
    phase_bytes(root, guests)?;
    Ok(())
}
fn snapshot(root: &Path, guest: &Value) -> Result<Vec<u8>, String> {
    fs::read(paths::existing(
        root,
        guest["data_disk"]["after"]["path"]
            .as_str()
            .ok_or("P07 snapshot")?,
    )?)
    .map_err(|e| e.to_string())
}
fn phase_bytes(root: &Path, guests: &[Value]) -> Result<(), String> {
    let saved = snapshot(root, &guests[0])?;
    let committed = snapshot(root, &guests[5])?;
    let log = super::journal::inspect(&committed)?;
    require(
        log.selected().committed && log.selected().entries.len() > 1,
        "P07 commit must cover multiple home pages",
    )?;
    let new_home = log.replayed_home(&committed)?;
    let home = HOME_FIRST as usize * 512..(HOME_FIRST + HOME_SECTORS) as usize * 512;
    let journal = LOG_FIRST as usize * 512..(LOG_FIRST + LOG_SECTORS) as usize * 512;
    let payload = journal.start + 8192..journal.start + 8192 + log.selected().entries.len() * 4096;
    for index in [0, 17] {
        let name = guests[index]["data_disk"]["path"]
            .as_str()
            .ok_or("P07 fixture path")?;
        let data = crate::storage::confined(root, name)?;
        let initial = data
            .parent()
            .ok_or("P07 fixture parent")?
            .join("initial.raw");
        paths::reject_links(root, &initial)?;
        let original = fs::read(initial).map_err(|e| e.to_string())?;
        let actual = snapshot(root, &guests[index])?;
        require(
            actual[..home.start] == original[..home.start]
                && actual[home.end..journal.start] == original[home.end..journal.start]
                && actual[journal.end..] == original[journal.end..],
            "P07 transaction modified unrelated partitions/gaps",
        )?;
    }
    for (index, stage) in [
        (2, "prepare"),
        (5, "commit"),
        (8, "first_home"),
        (11, "home"),
        (14, "checkpoint"),
    ] {
        let crash = snapshot(root, &guests[index])?;
        require(
            crash[..home.start] == saved[..home.start]
                && crash[home.end..journal.start] == saved[home.end..journal.start]
                && crash[journal.end..] == saved[journal.end..],
            "P07 crash changed bytes outside home and journal",
        )?;
        require(
            crash[payload.clone()] == committed[payload.clone()],
            "P07 stage did not persist the same prepared redo payload",
        )?;
        let mut expected = saved[home.clone()].to_vec();
        if stage == "first_home" {
            let first = log.selected().entries[0].0;
            expected[first * 4096..(first + 1) * 4096]
                .copy_from_slice(&committed[payload.start..payload.start + 4096]);
            require(
                expected != saved[home.clone()] && expected != new_home,
                "first-home fixture must change one page and leave another unapplied",
            )?;
        } else if matches!(stage, "home" | "checkpoint") {
            expected.copy_from_slice(&new_home);
        }
        require(
            crash[home.clone()] == expected,
            "P07 physical home bytes do not match interrupted commit point",
        )?;
        let recovered = snapshot(root, &guests[index + 1])?;
        require(
            recovered[..home.start] == saved[..home.start]
                && recovered[home.end..journal.start] == saved[home.end..journal.start]
                && recovered[journal.end..] == saved[journal.end..],
            "P07 recovery modified unrelated partitions/gaps",
        )?;
        let expected_home = if stage == "prepare" {
            &saved[home.clone()]
        } else {
            &new_home
        };
        require(
            &recovered[home.clone()] == expected_home,
            "P07 replay differs from independently reconstructed full home",
        )?;
        let normalized = super::journal::inspect(&recovered)?;
        require(
            normalized.headers[0] == normalized.headers[1] && !normalized.selected().committed,
            "P07 recovery did not normalize both checkpoint copies",
        )?;
    }
    Ok(())
}
