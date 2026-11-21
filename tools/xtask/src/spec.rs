use std::{
    fs,
    path::{Component, Path},
};

use serde_json::Value;

use crate::evidence::{file_hash, snapshot};
use crate::phase::Phase;

pub fn check(root: &Path) -> Result<(), String> {
    let baseline: Value = serde_json::from_slice(
        &fs::read(root.join("project/baseline.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let specification = root.join("DOOR_OS_MASTER_SPECIFICATION.md");
    if file_hash(&specification)?
        != baseline["specification_sha256"]
            .as_str()
            .ok_or("baseline hash missing")?
    {
        return Err("master specification differs from the supplied baseline; review and record an explicit revision".into());
    }
    let text = fs::read_to_string(&specification).map_err(|e| e.to_string())?;
    for section in 1..=144 {
        if !text.contains(&format!("<a id=\"section-{section}\"></a>")) {
            return Err(format!("specification section {section} missing"));
        }
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let required = baseline["required_paths"]
        .as_array()
        .ok_or("required_paths missing")?;
    for value in required {
        let name = value.as_str().ok_or("required path must be a string")?;
        checked_file(&root, name)?;
    }
    let dependencies: Value = serde_json::from_slice(
        &fs::read(root.join("specs/sources/dependencies.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if dependencies["lockfile_sha256"] != file_hash(&root.join("Cargo.lock"))? {
        return Err(
            "dependency lock changed; update the dependency review before passing P00".into(),
        );
    }
    let ledger: Value = serde_json::from_slice(
        &fs::read(root.join("project/phases.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let phases = ledger["phases"].as_array().ok_or("phase ledger missing")?;
    validate_active_phase(&ledger)?;
    if phases.len() != 30 {
        return Err("phase ledger must retain P00-P29".into());
    }
    let mut unavailable = Vec::new();
    let mut attested = Vec::new();
    for (index, phase) in phases.iter().enumerate() {
        if phase["id"] != format!("P{index:02}") {
            return Err("phase IDs are missing or out of order".into());
        }
        if ![
            "not_started",
            "in_progress",
            "blocked",
            "passed",
            "regression",
        ]
        .contains(&phase["phase_status"].as_str().unwrap_or(""))
        {
            return Err(format!("invalid phase state for P{index:02}"));
        }
        if !phase["parallel_authorization"].is_null() {
            checked_file(
                &root,
                phase["parallel_authorization"]["decision_record"]
                    .as_str()
                    .ok_or("parallel authorization decision record missing")?,
            )?;
        }
        if phase["phase_status"] == "passed" {
            match validate_passed_phase(&root, &ledger, phase)? {
                Evidence::OwnerConfirmedUnavailable => {
                    unavailable.push(phase["id"].as_str().unwrap_or_default());
                }
                Evidence::OwnerAttested => attested.push(phase["id"].as_str().unwrap_or_default()),
                Evidence::Revalidated => {}
            }
        }
    }
    println!(
        "Specification preserved; {} required artifacts and 30 phase records checked.",
        required.len()
    );
    if !unavailable.is_empty() {
        println!(
            "Owner-confirmed unavailable historical reports (identity and critic checked, not reopened; ADR 0017): {}.",
            unavailable.join(", ")
        );
    }
    if !attested.is_empty() {
        println!(
            "Owner-attested passes without retained evidence (no report or critic exists; ADR 0027): {}.",
            attested.join(", ")
        );
    }
    Ok(())
}

/// How a passed phase's evidence was established by `check`.
#[derive(Debug, PartialEq, Eq)]
enum Evidence {
    /// The qualifying report, its logs and its critic were fully revalidated.
    Revalidated,
    /// The report is absent under a reviewed owner disposition; only its original
    /// identity and the critic file that reviewed it were checked. Nothing was
    /// reopened, recreated or requalified under the original run ID.
    OwnerConfirmedUnavailable,
    /// No report or critic was ever recorded: the owner attested the pass
    /// (ADR 0027, P12-P14 only). Later cumulative gates re-execute its checks.
    OwnerAttested,
}

/// Owner decision of 11 November 2026 (ADR 0027): P12-P14 passed and their
/// evidence was deleted before it was recorded.
const ATTESTED_STATE: &str = "passed_owner_attested";
const ATTESTED_DATE: &str = "2026-11-11";
const ATTESTED_RECORD: &str = "docs/adr/0027-p12-p14-owner-attested-pass.md";
const ATTESTED_PHASES: [Phase; 3] = [Phase::P12, Phase::P13, Phase::P14];

/// An ADR 0027 attestation: exactly the reviewed fields, P12-P14 only, never
/// the selected phase, and no report or critic claimed.
fn validate_attestation(root: &Path, phase: &Value, current: bool) -> Result<(), String> {
    let id = phase["id"].as_str().ok_or("phase ID missing")?;
    let selected = Phase::parse(id)?;
    let disposition = &phase["evidence_disposition"];
    if current {
        return Err(format!(
            "{id} is the selected phase; an owner attestation cannot replace current qualifying evidence"
        ));
    }
    if !ATTESTED_PHASES.contains(&selected) {
        return Err(format!(
            "{id}: an owner-attested pass is approved only for P12-P14 ({ATTESTED_RECORD})"
        ));
    }
    if disposition["owner_confirmed"] != true
        || disposition["date"] != ATTESTED_DATE
        || disposition["owner_statement"]
            .as_str()
            .is_none_or(|statement| statement.trim().len() < 20)
        || disposition["decision_record"] != ATTESTED_RECORD
        || !phase["evidence_report"].is_null()
        || !phase["critic_review"].is_null()
    {
        return Err(format!(
            "{id} owner attestation is not the reviewed record (owner_confirmed, date, owner_statement and decision_record required; no evidence_report or critic_review may be claimed)"
        ));
    }
    checked_file(root, ATTESTED_RECORD)?;
    Ok(())
}

/// Owner decision 1 (23 October 2026, ADR 0017): the P00-P11 qualifying reports
/// were deleted by the owner for disk space. Only these exact reviewed fields let
/// `check` accept a missing historical report; anything else stays strict.
const DISPOSITION_STATE: &str = "unavailable_owner_confirmed";
const DISPOSITION_DATE: &str = "2026-10-23";
const DISPOSITION_RECORD: &str = "docs/adr/0017-historical-evidence-disposition.md";
const DISPOSITION_THROUGH: Phase = Phase::P11;

fn validate_passed_phase(root: &Path, ledger: &Value, phase: &Value) -> Result<Evidence, String> {
    let id = phase["id"].as_str().ok_or("phase ID missing")?;
    Phase::parse(id)?;
    let current = ledger["active_phase"] == phase["id"];
    if phase["evidence_disposition"]["state"] == ATTESTED_STATE {
        validate_attestation(root, phase, current)?;
        return Ok(Evidence::OwnerAttested);
    }
    let evidence = phase["evidence_report"]
        .as_str()
        .ok_or("passed phase needs evidence_report")?;
    let disposed = !phase["evidence_disposition"].is_null();
    if disposed {
        validate_disposition(root, phase, current)?;
    }
    if !evidence_present(root, evidence)? {
        if disposed {
            return Ok(Evidence::OwnerConfirmedUnavailable);
        }
        return Err(format!(
            "{id} evidence report {evidence} is unavailable and the phase has no reviewed evidence_disposition; restore the report or rerun the gate"
        ));
    }
    // A disposition never weakens a check that can still be performed: a present
    // report is fully revalidated exactly as before.
    let report_path = checked_file(root, evidence)?;
    let report: Value = serde_json::from_slice(&fs::read(&report_path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    validate_phase_gate(root, &report, id, current)?;
    let critic = read_critic(root, phase)?;
    validate_critic(&critic, &report, evidence, &file_hash(&report_path)?)?;
    let commands = report["commands"]
        .as_array()
        .ok_or("evidence has no commands")?;
    if commands.is_empty() {
        return Err("passed phase references an empty run".into());
    }
    for command in commands {
        if command["required"] == true && command["outcome"] != "passed" {
            return Err("mandatory command failed".into());
        }
        for stream in ["stdout", "stderr"] {
            let path = checked_file(
                root,
                command[stream]["path"].as_str().ok_or("log path missing")?,
            )?;
            if file_hash(&path)?
                != command[stream]["sha256"]
                    .as_str()
                    .ok_or("log hash missing")?
            {
                return Err("evidence log hash mismatch".into());
            }
        }
    }
    Ok(Evidence::Revalidated)
}

fn read_critic(root: &Path, phase: &Value) -> Result<Value, String> {
    serde_json::from_slice(
        &fs::read(checked_file(
            root,
            phase["critic_review"]
                .as_str()
                .ok_or("passed phase needs critic_review")?,
        )?)
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Path syntax is checked before existence so a traversal can never be treated
/// as an unavailable report.
fn evidence_present(root: &Path, name: &str) -> Result<bool, String> {
    let relative = Path::new(name);
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("invalid relative path: {name}"));
    }
    Ok(fs::symlink_metadata(root.join(relative)).is_ok())
}

fn validate_disposition(root: &Path, phase: &Value, current: bool) -> Result<(), String> {
    let id = phase["id"].as_str().ok_or("phase ID missing")?;
    let selected = Phase::parse(id)?;
    let disposition = &phase["evidence_disposition"];
    if current {
        return Err(format!(
            "{id} is the selected phase; a historical evidence disposition cannot replace current qualifying evidence"
        ));
    }
    if selected > DISPOSITION_THROUGH {
        return Err(format!(
            "{id}: the owner-confirmed evidence disposition is approved only for P00-P11 ({DISPOSITION_RECORD})"
        ));
    }
    if disposition["state"] != DISPOSITION_STATE
        || disposition["owner_confirmed"] != true
        || disposition["date"] != DISPOSITION_DATE
        || disposition["owner_statement"]
            .as_str()
            .is_none_or(|statement| statement.trim().len() < 20)
        || disposition["decision_record"] != DISPOSITION_RECORD
    {
        return Err(format!(
            "{id} evidence_disposition is not the reviewed owner-confirmed record (state, owner_confirmed, date, owner_statement and decision_record are required)"
        ));
    }
    checked_file(root, DISPOSITION_RECORD)?;
    let evidence = phase["evidence_report"]
        .as_str()
        .ok_or("passed phase needs evidence_report")?;
    let run_id = evidence
        .strip_prefix("reports/runs/")
        .and_then(|rest| rest.strip_suffix("/report.json"))
        .filter(|run| crate::layout::canonical_owner(run))
        .ok_or_else(|| format!("{id} original evidence identity is not a canonical run report"))?;
    if disposition["original_run_id"] != run_id {
        return Err(format!(
            "{id} evidence_disposition run ID differs from the ledger's original report identity"
        ));
    }
    // The critic file is retained historical identity: it must exist and bind
    // the same original run, phase and a passed, finding-free review.
    let critic = read_critic(root, phase)?;
    if critic["outcome"] != "passed"
        || critic["unresolved_findings"] != 0
        || critic["evidence_report"] != evidence
        || critic["phase"] != id
        || critic["agent"].as_str().is_none_or(str::is_empty)
        || critic["reviewed_build_inputs"]
            .as_array()
            .is_none_or(Vec::is_empty)
    {
        return Err(format!(
            "{id} critic review does not bind the original run, phase and reviewed inputs"
        ));
    }
    let reviewed = critic["evidence_report_sha256"]
        .as_str()
        .or_else(|| critic["qualifying_report_sha256"].as_str());
    let recorded = &disposition["original_report_sha256"];
    let digest_ok = match reviewed {
        Some(digest) => {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && *recorded == digest
        }
        // P00-P02 critics predate report digests; nothing unverifiable may be added.
        None => selected < Phase::P03 && recorded.is_null(),
    };
    if !digest_ok {
        return Err(format!(
            "{id} evidence_disposition report digest differs from the critic's reviewed report"
        ));
    }
    Ok(())
}

/// Owner decision 6 (23 October 2026): P13 may proceed in parallel with the
/// selected P12. A later phase may be in progress only with this exact record.
fn validate_parallel_authorization(phase: &Value, active: &str) -> Result<(), String> {
    let id = phase["id"].as_str().ok_or("phase ID missing")?;
    Phase::parse(id)?;
    let authorization = &phase["parallel_authorization"];
    if authorization["with"] != active
        || authorization["owner_confirmed"] != true
        || authorization["date"]
            .as_str()
            .is_none_or(|date| !iso_date(date))
        || authorization["owner_decision"]
            .as_str()
            .is_none_or(|decision| decision.trim().len() < 20)
        || authorization["decision_record"]
            .as_str()
            .is_none_or(str::is_empty)
    {
        return Err(format!(
            "{id} lacks an owner-confirmed parallel authorization with the selected phase {active}"
        ));
    }
    Ok(())
}

/// Phases a later phase's cumulative report may certify (ADR 0027), and
/// that phase.
const CUMULATIVE_CITATIONS: [(&str, &str); 2] = [("P15", "P17"), ("P16", "P17")];
fn cites_cumulative(phase: &str, report: &Value) -> bool {
    CUMULATIVE_CITATIONS
        .iter()
        .any(|(earlier, later)| *earlier == phase && report["certified_phase"] == *later)
}

fn iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    let digits = |range: std::ops::Range<usize>| bytes[range].iter().all(u8::is_ascii_digit);
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && digits(0..4)
        && digits(5..7)
        && digits(8..10)
        && (1..=12).contains(&value[5..7].parse::<u8>().unwrap_or(0))
        && (1..=31).contains(&value[8..10].parse::<u8>().unwrap_or(0))
}

/// The selected phase plus every owner-authorized parallel phase, in order.
pub fn active_phases(ledger: &Value) -> Result<Vec<String>, String> {
    validate_active_phase(ledger)?;
    let active = ledger["active_phase"]
        .as_str()
        .ok_or("active_phase missing")?;
    let phases = ledger["phases"].as_array().ok_or("phase ledger missing")?;
    let index = phases
        .iter()
        .position(|phase| phase["id"] == active)
        .ok_or("active_phase must identify a declared phase")?;
    let mut selected = vec![active.to_owned()];
    for phase in &phases[index + 1..] {
        if phase["phase_status"] == "not_started" {
            break;
        }
        selected.push(phase["id"].as_str().ok_or("phase ID missing")?.to_owned());
    }
    Ok(selected)
}

/// The phase a verify run certifies: the ledger's selected phase by default, or
/// an explicitly requested owner-authorized parallel phase.
pub fn verification_phase(ledger: &Value, requested: Option<&str>) -> Result<String, String> {
    let phases = active_phases(ledger)?;
    match requested {
        None => Ok(phases[0].clone()),
        Some(name) if phases.iter().any(|phase| phase == name) => {
            Phase::parse(name)?;
            Ok(name.to_owned())
        }
        Some(name) => Err(format!(
            "{name} is neither the selected phase nor an owner-authorized parallel phase (active: {})",
            phases.join(", ")
        )),
    }
}

#[cfg(test)]
fn validate_gate(root: &Path, report: &Value) -> Result<(), String> {
    validate_phase_gate(root, report, "P00", true)
}

fn validate_phase_gate(
    root: &Path,
    report: &Value,
    phase: &str,
    current: bool,
) -> Result<(), String> {
    let selected = Phase::parse(phase)?;
    if report["success"] != true
        || report["action"] != serde_json::json!(["verify"])
        || report["inputs_unchanged"] != true
    {
        return Err("passed phase requires a successful full verify run".into());
    }
    // P00 predates certified_phase. Later reports cannot select a weaker gate.
    // P15 and P16 may cite P17's cumulative report (owner decision of 11
    // November 2026, ADR 0027): it re-executes every P15 and P16 command
    // and scenario, which the containment check below still requires.
    if phase != "P00" && report["certified_phase"] != phase && !cites_cumulative(phase, report) {
        return Err("report certification differs from selected phase".into());
    }
    let commands = report["commands"]
        .as_array()
        .ok_or("evidence has no commands")?;
    for argv in crate::phase_commands(phase) {
        if !commands.iter().any(|command| {
            command["argv"] == serde_json::json!(argv)
                && successful_command_fields(command, phase == "P00" && !current, 0)
        }) {
            return Err("passed phase is missing a mandatory gate command".into());
        }
    }
    if phase == "P01" {
        if report["certified_phase"] != "P01" {
            return Err("P01 requires a P01 certified run".into());
        }
        let checks = report["checks"]
            .as_array()
            .ok_or("P01 check records missing")?;
        for id in ["p01-build-plan", "p01-reproducible-host-build"] {
            if !checks
                .iter()
                .any(|check| check["id"] == id && check["outcome"] == "passed")
            {
                return Err(format!("missing mandatory P01 check {id}"));
            }
        }
        if current {
            let hardware =
                crate::image::read_json(&root.join("profiles/hardware/qemu-diagnostic-x64.json"))?;
            if hardware["runnable"] == true {
                let (qemu, _, _) = crate::emulator::acquired_inputs(&hardware)?;
                if !checks.iter().any(|check| {
                    check["id"] == "p01-emulator-prerequisites"
                        && check["outcome"] == "passed"
                        && check["details"] == hardware
                }) {
                    return Err("resolved P01 requires its exact emulator preflight check".into());
                }
                validate_preflight_commands(commands, qemu.to_str().ok_or("QEMU path")?)?;
            }
        }
        let rebuild = checks
            .iter()
            .find(|check| check["id"] == "p01-reproducible-host-build")
            .ok_or("rebuild check missing")?;
        let artifacts = rebuild["details"]["artifacts"]
            .as_array()
            .ok_or("rebuild artifacts missing")?;
        if artifacts.len() != 2 || artifacts[0]["sha256"] != artifacts[1]["sha256"] {
            return Err("controlled rebuild evidence does not match".into());
        }
        validate_rebuild_identity(root, artifacts, commands, &report["source_snapshot"])?;
        for artifact in artifacts {
            let name = artifact["path"]
                .as_str()
                .ok_or("built artifact path missing")?;
            if !name.starts_with("out/builds/") {
                return Err("build artifact outside build output root".into());
            }
            let path = crate::paths::existing(root, name)?;
            if crate::evidence::file_hash(&path)?
                != artifact["sha256"]
                    .as_str()
                    .ok_or("artifact digest missing")?
            {
                return Err("built artifact bytes changed".into());
            }
            let executable = path.to_str().ok_or("artifact path not UTF8")?;
            if !commands.iter().any(|command| {
                command["argv"] == serde_json::json!([executable, "--help"])
                    && command["outcome"] == "passed"
            }) {
                return Err("built host executable was not smoke tested".into());
            }
        }
    }
    if selected >= Phase::P02 {
        crate::boot_gate::validate(root, report, phase)?;
    }
    if selected >= Phase::P10 && current {
        crate::session::verify::validate(root, report)?;
    }
    if selected >= Phase::P11 && current {
        crate::desktop::validate(root, report)?;
    }
    if selected >= Phase::P12 && current {
        // Every cumulative P12 (for P13 Windows GUI, for P14 desktop) acceptance suite needs
        // its own passed check; a missing runner can never produce one.
        crate::scenarios::validate_acceptance(report, phase)?;
        // Every existing desktop scenario beyond the P11 desktop gate.
        crate::scenarios::validate_desktop_scenarios(root, report, phase)?;
    }
    let recorded = build_inputs_version(&report["source_snapshot"], phase != "P00")?;
    if current && recorded != build_inputs_version(&snapshot(root)?, phase != "P00")? {
        return Err("phase evidence is stale for current code/build/contract inputs; set phase in_progress and rerun verify".into());
    }
    Ok(())
}

fn validate_rebuild_identity(
    root: &Path,
    artifacts: &[Value],
    commands: &[Value],
    snapshot: &Value,
) -> Result<(), String> {
    if artifacts.len() != 2 || artifacts[0]["path"] == artifacts[1]["path"] {
        return Err("two distinct rebuilt artifact paths required".into());
    }
    let mut targets = Vec::new();
    for artifact in artifacts {
        let name = artifact["path"].as_str().ok_or("artifact path missing")?;
        let path = crate::paths::existing(root, name)?;
        let target = path.ancestors().nth(3).ok_or("build directory missing")?;
        if targets.contains(&target.to_path_buf()) {
            return Err("independent build directories required".into());
        }
        targets.push(target.to_path_buf());
        let target_name = target.to_str().ok_or("build directory not UTF8")?;
        let build = serde_json::json!([
            "cargo",
            "build",
            "--locked",
            "--release",
            "--package",
            "door-xtask",
            "--bin",
            "door-xtask",
            "--target",
            "x86_64-pc-windows-msvc",
            "--target-dir",
            target_name
        ]);
        if !commands
            .iter()
            .any(|command| command["argv"] == build && successful_command(command))
        {
            return Err("each artifact needs its own successful controlled build command".into());
        }
        if artifact["build_inputs"] != *snapshot {
            return Err("build artifact source identity differs from qualifying run".into());
        }
        let mut inspected = crate::artifact::inspect(&path, "host")?;
        inspected["execution_state"] = serde_json::json!("host_smoke_tested");
        let mut recorded = artifact.clone();
        let object = recorded
            .as_object_mut()
            .ok_or("artifact must be an object")?;
        object.remove("path");
        object.remove("build_inputs");
        if inspected != recorded {
            return Err("built artifact inspection does not match current file bytes".into());
        }
        let smoke = serde_json::json!([path.to_str().ok_or("artifact path not UTF8")?, "--help"]);
        if !commands
            .iter()
            .any(|command| command["argv"] == smoke && successful_command(command))
        {
            return Err("each artifact needs its own successful smoke command".into());
        }
    }
    Ok(())
}

pub fn verify_current_rebuild(run: &crate::evidence::Run) -> Result<(), String> {
    let check = run
        .checks()
        .iter()
        .find(|check| check["id"] == "p01-reproducible-host-build")
        .ok_or("rebuild evidence missing")?;
    let artifacts = check["details"]["artifacts"]
        .as_array()
        .ok_or("rebuild artifacts missing")?;
    validate_rebuild_identity(&run.root, artifacts, run.commands(), &run.build_inputs())
}
pub(crate) fn successful_command(command: &Value) -> bool {
    successful_command_at_exit(command, 0)
}

pub(crate) fn successful_command_at_exit(command: &Value, expected: i32) -> bool {
    successful_command_fields(command, false, expected)
}

fn successful_command_fields(command: &Value, historical_p00: bool, expected: i32) -> bool {
    command["required"] == true
        && command["outcome"] == "passed"
        && command["exit_code"] == expected
        && command["expected_exit_code"] == expected
        && command["timed_out"] == false
        && command.get("spawn_error") == Some(&Value::Null)
        && command.get("cleanup_error") == Some(&Value::Null)
        && command["output_budget_error"].is_null()
        // P00 predates this field. Preserve only its immutable historical schema;
        // current runs must explicitly record a false output-limit result.
        && (command["output_limit_exceeded"] == false
            || (historical_p00 && command.get("output_limit_exceeded").is_none()))
}

fn validate_preflight_commands(commands: &[Value], qemu: &str) -> Result<(), String> {
    for args in [
        vec!["--version"],
        vec!["-machine", "help"],
        vec!["-cpu", "help"],
        vec!["-accel", "help"],
    ] {
        let argv: Vec<_> = core::iter::once(qemu).chain(args).collect();
        if !commands.iter().any(|command| {
            command["argv"] == serde_json::json!(argv) && successful_command(command)
        }) {
            return Err(
                "resolved P01 is missing successful acquired emulator preflight commands".into(),
            );
        }
    }
    Ok(())
}

fn build_inputs(manifest: &Value) -> Result<Vec<Value>, String> {
    build_inputs_version(manifest, false)
}
fn build_inputs_version(manifest: &Value, include_hardware: bool) -> Result<Vec<Value>, String> {
    Ok(manifest["files"]
        .as_array()
        .ok_or("source snapshot missing")?
        .iter()
        .filter(|file| {
            file["path"].as_str().is_some_and(|path| {
                path.ends_with(".rs")
                    || path.ends_with(".c")
                    || path.ends_with(".asm")
                    || path.ends_with(".h")
                    || path.ends_with(".toml")
                    || path == "Cargo.lock"
                    || path == "DOOR_OS_MASTER_SPECIFICATION.md"
                    || path.starts_with("specs/")
                    || path.starts_with("third_party/apps/")
                    || path.starts_with("assets/")
                    || path.starts_with("profiles/build/")
                    || (include_hardware && path.starts_with("profiles/hardware/"))
                    || (include_hardware
                        && (path.ends_with(".S")
                            || path.ends_with(".ld")
                            || path.starts_with("boot/config/")))
            })
        })
        .cloned()
        .collect())
}

fn validate_active_phase(ledger: &Value) -> Result<(), String> {
    let phases = ledger["phases"].as_array().ok_or("phase ledger missing")?;
    let active = ledger["active_phase"]
        .as_str()
        .ok_or("active_phase missing")?;
    let index = phases
        .iter()
        .position(|phase| phase["id"] == active)
        .ok_or("active_phase must identify a declared phase")?;
    // Owner-authorized parallel phases must directly follow the selected phase
    // without gaps, stay in progress or blocked, and can never pass first.
    let mut parallel_chain = true;
    for (position, phase) in phases.iter().enumerate() {
        let state = phase["phase_status"]
            .as_str()
            .ok_or("phase_status missing")?;
        if position > index && state != "not_started" {
            if !parallel_chain || !matches!(state, "in_progress" | "blocked") {
                return Err("later phases must remain not_started unless owner-authorized to run in parallel with the selected phase".into());
            }
            validate_parallel_authorization(phase, active)?;
            continue;
        }
        if position > index {
            parallel_chain = false;
        }
        if matches!(state, "in_progress" | "blocked" | "regression") && position != index {
            return Err("only the selected phase may be active, blocked or under repair".into());
        }
        if position < index && state != "passed" {
            return Err("earlier phases must pass before beginning the selected phase".into());
        }
    }
    Ok(())
}

fn validate_critic(
    critic: &Value,
    report: &Value,
    evidence_path: &str,
    report_sha256: &str,
) -> Result<(), String> {
    let selected = report["certified_phase"]
        .as_str()
        .map(Phase::parse)
        .transpose()?;
    if critic["outcome"] != "passed"
        || critic["unresolved_findings"] != 0
        || critic["evidence_report"] != evidence_path
        || (selected.is_some_and(|p| p >= Phase::P03)
            && critic["evidence_report_sha256"] != report_sha256)
        || (selected.is_some_and(|p| p >= Phase::P03)
            && critic["phase"] != report["certified_phase"])
        || critic["reviewed_build_inputs"]
            != serde_json::json!(if selected.is_some_and(|p| p >= Phase::P01) {
                build_inputs_version(&report["source_snapshot"], true)?
            } else {
                build_inputs(&report["source_snapshot"])?
            })
        || critic["agent"].as_str().is_none_or(|name| name.is_empty())
    {
        return Err(
            "critic review is incomplete or does not cover the qualifying run/source inputs".into(),
        );
    }
    Ok(())
}

fn checked_file(root: &Path, name: &str) -> Result<std::path::PathBuf, String> {
    let relative = Path::new(name);
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("invalid relative path: {name}"));
    }
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|e| format!("{name}: {e}"))?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(format!("path escapes repository or is not a file: {name}"));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_c_masm_and_headers_are_critic_and_closure_inputs() {
        let files = serde_json::json!([
            {"path":"tests/fixtures/windows-x64/main.c","sha256":"c"},
            {"path":"tests/fixtures/windows-x64/abi.asm","sha256":"asm"},
            {"path":"tests/fixtures/windows-x64/contract.h","sha256":"header"}
        ]);
        let snapshot = serde_json::json!({"files":files});
        assert_eq!(
            super::build_inputs_version(&snapshot, true).unwrap(),
            files.as_array().unwrap().clone()
        );
        let report = serde_json::json!({"certified_phase":"P09","source_snapshot":snapshot});
        let critic = serde_json::json!({"phase":"P09","outcome":"passed","unresolved_findings":0,"agent":"independent","evidence_report":"reports/qualified.json","evidence_report_sha256":"exact","reviewed_build_inputs":files});
        assert!(
            super::validate_critic(&critic, &report, "reports/qualified.json", "exact").is_ok()
        );
        for index in 0..3 {
            let mut changed = critic.clone();
            changed["reviewed_build_inputs"][index]["sha256"] = serde_json::json!("different");
            assert!(
                super::validate_critic(&changed, &report, "reports/qualified.json", "exact")
                    .is_err()
            );
        }
    }
    use super::*;

    #[test]
    fn evidence_paths_reject_traversal_and_absolute_paths() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        for name in [
            "../Cargo.toml",
            "./Cargo.toml",
            "C:/Windows/win.ini",
            "/etc/passwd",
        ] {
            assert!(checked_file(&root, name).is_err(), "{name}");
        }
        assert!(checked_file(&root, "Cargo.toml").is_ok());
    }

    #[test]
    fn a_doctor_run_or_empty_verify_cannot_qualify_a_phase() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let doctor = serde_json::json!({"success": true, "action": ["doctor"], "commands": []});
        assert!(validate_gate(root, &doctor).is_err());
        let empty = serde_json::json!({"success": true, "action": ["verify"], "commands": []});
        assert!(validate_gate(root, &empty).is_err());
    }

    #[test]
    fn a_failed_or_timed_out_mandatory_command_cannot_qualify_a_phase() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let commands: Vec<_> = crate::gate_commands()
            .into_iter()
            .map(|argv| {
                serde_json::json!({"argv": argv,
            "required": true, "outcome": "passed", "exit_code": 0, "timed_out": true})
            })
            .collect();
        let report = serde_json::json!({"success": true, "inputs_unchanged": true, "action": ["verify"], "commands": commands});
        assert!(validate_gate(root, &report).is_err());
    }

    #[test]
    fn mandatory_receipts_reject_contradictions_and_preserve_only_historical_p00_schema() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let commands: Vec<_> = crate::phase_commands("P00")
            .into_iter()
            .map(|argv| {
                serde_json::json!({"argv":argv,"required":true,
                "outcome":"passed","exit_code":0,"expected_exit_code":0,
                "timed_out":false,"output_limit_exceeded":false,
                "spawn_error":null,"cleanup_error":null})
            })
            .collect();
        let good = serde_json::json!({"success":true,"inputs_unchanged":true,
            "action":["verify"],"commands":commands,"source_snapshot":{"files":[]}});
        assert!(validate_phase_gate(root, &good, "P00", false).is_ok());
        for (field, value) in [
            ("required", serde_json::json!(false)),
            ("outcome", serde_json::json!("failed")),
            ("exit_code", serde_json::json!(1)),
            ("expected_exit_code", serde_json::json!(1)),
            ("timed_out", serde_json::json!(true)),
            ("output_limit_exceeded", serde_json::json!(true)),
            ("spawn_error", serde_json::json!("spawn failed")),
            ("cleanup_error", serde_json::json!("cleanup failed")),
        ] {
            let mut bad = good.clone();
            bad["commands"][0][field] = value;
            for current in [false, true] {
                assert_eq!(
                    validate_phase_gate(root, &bad, "P00", current),
                    Err("passed phase is missing a mandatory gate command".into()),
                    "{field}"
                );
            }
        }
        for field in [
            "required",
            "outcome",
            "exit_code",
            "expected_exit_code",
            "timed_out",
            "output_limit_exceeded",
            "spawn_error",
            "cleanup_error",
        ] {
            let mut bad = good.clone();
            bad["commands"][0].as_object_mut().unwrap().remove(field);
            assert_eq!(
                validate_phase_gate(root, &bad, "P00", true),
                Err("passed phase is missing a mandatory gate command".into()),
                "{field}"
            );
            assert_eq!(
                validate_phase_gate(root, &bad, "P00", false).is_ok(),
                field == "output_limit_exceeded"
            );
        }
        let mut legacy = good["commands"][0].clone();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("output_limit_exceeded");
        assert!(!successful_command(&legacy));
        legacy["output_limit_exceeded"] = Value::Null;
        assert!(!successful_command_fields(&legacy, true, 0));
    }

    #[test]
    fn phase_selection_rejects_unknown_parallel_and_skipped_phases() {
        let mut ledger = serde_json::json!({"active_phase": "P00", "phases": [
            {"id": "P00", "phase_status": "in_progress"}, {"id": "P01", "phase_status": "not_started"}]});
        assert!(validate_active_phase(&ledger).is_ok());
        ledger["active_phase"] = serde_json::json!("P99");
        assert!(validate_active_phase(&ledger).is_err());
        ledger["active_phase"] = serde_json::json!("P00");
        ledger["phases"][1]["phase_status"] = serde_json::json!("in_progress");
        assert!(validate_active_phase(&ledger).is_err());
        ledger["active_phase"] = serde_json::json!("P01");
        assert!(validate_active_phase(&ledger).is_err());
        ledger["phases"][0]["phase_status"] = serde_json::json!("passed");
        assert!(validate_active_phase(&ledger).is_ok());
    }

    #[test]
    fn critic_acceptance_requires_the_reviewed_run_and_inputs() {
        let report = serde_json::json!({"source_snapshot": {"files": [{"path": "source.rs", "sha256": "original"}]}});
        let mut critic = serde_json::json!({"outcome": "passed", "unresolved_findings": 0, "agent": "test-critic",
            "evidence_report": "reports/run.json", "reviewed_build_inputs": [{"path": "source.rs", "sha256": "original"}]});
        assert!(validate_critic(&critic, &report, "reports/run.json", "digest").is_ok());
        assert!(validate_critic(&critic, &report, "reports/another-run.json", "digest").is_err());
        critic["reviewed_build_inputs"][0]["sha256"] = serde_json::json!("modified");
        assert!(validate_critic(&critic, &report, "reports/run.json", "digest").is_err());
        critic["reviewed_build_inputs"][0]["sha256"] = serde_json::json!("original");
        let mut current = report.clone();
        current["certified_phase"] = serde_json::json!("P03");
        critic["phase"] = serde_json::json!("P03");
        assert!(validate_critic(&critic, &current, "reports/run.json", "digest").is_err());
        critic["evidence_report_sha256"] = serde_json::json!("digest");
        assert!(validate_critic(&critic, &current, "reports/run.json", "digest").is_ok());
        assert!(validate_critic(&critic, &current, "reports/run.json", "changed").is_err());
        critic["phase"] = serde_json::json!("P02");
        assert!(validate_critic(&critic, &current, "reports/run.json", "digest").is_err());
    }

    #[test]
    fn older_certification_cannot_select_a_weaker_historical_gate() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let report = serde_json::json!({"success":true,"action":["verify"],
            "inputs_unchanged":true,"certified_phase":"P02"});
        for phase in ["P03", "P04", "P05", "P06", "P07", "P08"] {
            for current in [false, true] {
                assert_eq!(
                    validate_phase_gate(root, &report, phase, current),
                    Err("report certification differs from selected phase".into())
                );
            }
        }
    }

    #[test]
    fn p04_critic_needs_current_phase_report_digest_and_profile_inputs() {
        let inputs = serde_json::json!([
            {"path":"kernel/src/arch/x86_64/ap_start.S","sha256":"assembly"},
            {"path":"profiles/hardware/qemu-smp2-x64.json","sha256":"profile"}
        ]);
        let report =
            serde_json::json!({"certified_phase":"P04", "source_snapshot":{"files":inputs}});
        let good = serde_json::json!({"outcome":"passed","unresolved_findings":0,"agent":"critic",
            "phase":"P04","evidence_report":"reports/run.json","evidence_report_sha256":"exact",
            "reviewed_build_inputs":inputs});
        assert!(validate_critic(&good, &report, "reports/run.json", "exact").is_ok());
        for (field, value) in [
            ("phase", serde_json::json!("P03")),
            ("evidence_report_sha256", Value::Null),
            ("evidence_report", serde_json::json!("reports/older.json")),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert!(validate_critic(&bad, &report, "reports/run.json", "exact").is_err());
        }
        let mut bad = good;
        bad["reviewed_build_inputs"][1]["sha256"] = serde_json::json!("changed topology");
        assert!(validate_critic(&bad, &report, "reports/run.json", "exact").is_err());
        assert!(validate_phase_gate(Path::new("."), &serde_json::json!({
            "success":true,"action":["verify"],"inputs_unchanged":true,"certified_phase":"P18"
        }), "P18", false).unwrap_err().contains("not implemented"));
        // P12-P14 are explicit schemas now, but an empty run cannot qualify them.
        for phase in ["P12", "P13", "P14"] {
            assert_eq!(
                validate_phase_gate(
                    Path::new("."),
                    &serde_json::json!({
                        "success":true,"action":["verify"],"inputs_unchanged":true,
                        "certified_phase":phase,"commands":[]
                    }),
                    phase,
                    true
                ),
                Err("passed phase is missing a mandatory gate command".into())
            );
        }
    }

    /// ADR 0027: P15 and P16 may cite P17's cumulative report; nothing else
    /// may cite another phase's, and the cited report must still contain
    /// every command of the citing phase.
    #[test]
    fn only_p15_and_p16_cite_the_p17_cumulative_report() {
        let report = |certified: &str| {
            serde_json::json!({"success":true,"action":["verify"],"inputs_unchanged":true,
                "certified_phase":certified,"commands":[]})
        };
        let missing = Err(String::from(
            "passed phase is missing a mandatory gate command",
        ));
        let differs = Err(String::from(
            "report certification differs from selected phase",
        ));
        for phase in ["P15", "P16"] {
            // Accepted as a citation: the containment check runs next.
            assert_eq!(
                validate_phase_gate(Path::new("."), &report("P17"), phase, false),
                missing
            );
            assert_eq!(
                validate_phase_gate(Path::new("."), &report("P18"), phase, false),
                differs
            );
        }
        assert_eq!(
            validate_phase_gate(Path::new("."), &report("P17"), "P14", false),
            differs
        );
        assert_eq!(
            validate_phase_gate(Path::new("."), &report("P16"), "P15", false),
            differs
        );
    }

    #[test]
    fn p05_critic_and_commands_bind_native_schema_and_independent_caller() {
        let inputs = serde_json::json!([
            {"path":"runtime/abi/src/lib.rs","sha256":"schema"},
            {"path":"specs/abi/native-x64-v1.md","sha256":"contract"},
            {"path":"tests/fixtures/native/src/probe.S","sha256":"independent"},
            {"path":"tests/fixtures/native/linker.ld","sha256":"linker"}
        ]);
        let report =
            serde_json::json!({"certified_phase":"P05","source_snapshot":{"files":inputs}});
        let good = serde_json::json!({"outcome":"passed","unresolved_findings":0,"agent":"critic",
            "phase":"P05","evidence_report":"reports/run.json","evidence_report_sha256":"exact","reviewed_build_inputs":inputs});
        assert!(validate_critic(&good, &report, "reports/run.json", "exact").is_ok());
        for field in ["phase", "evidence_report_sha256", "reviewed_build_inputs"] {
            let mut bad = good.clone();
            bad[field] = Value::Null;
            assert!(validate_critic(&bad, &report, "reports/run.json", "exact").is_err());
        }
        let commands = crate::phase_commands("P05");
        for required in crate::phase_commands("P04") {
            assert!(commands.contains(&required));
        }
        assert!(
            commands
                .iter()
                .any(|argv| argv.contains(&"door-native-fixtures")
                    && argv.contains(&"clippy")
                    && argv.contains(&"warnings"))
        );
        for package in ["door-abi", "door-sys", "door-rt"] {
            assert!(
                commands
                    .iter()
                    .any(|argv| argv.contains(&package) && argv.contains(&"x86_64-unknown-none"))
            );
        }
    }

    #[test]
    fn p06_critic_is_bound_to_its_storage_profile_and_exact_phase_digest() {
        let inputs = serde_json::json!([
            {"path":"tools/xtask/src/storage/audit.rs","sha256":"auditor"},
            {"path":"profiles/hardware/qemu-storage-x64.json","sha256":"profile"},
            {"path":"specs/contracts/storage-filesystems-v1.md","sha256":"contract"}
        ]);
        let report =
            serde_json::json!({"certified_phase":"P06","source_snapshot":{"files":inputs}});
        let good = serde_json::json!({"outcome":"passed","unresolved_findings":0,"agent":"critic","phase":"P06","evidence_report":"reports/run.json","evidence_report_sha256":"exact","reviewed_build_inputs":inputs});
        assert!(validate_critic(&good, &report, "reports/run.json", "exact").is_ok());
        for (field, value) in [
            ("phase", serde_json::json!("P05")),
            ("evidence_report_sha256", Value::Null),
            ("reviewed_build_inputs", serde_json::json!([])),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert!(validate_critic(&bad, &report, "reports/run.json", "exact").is_err());
        }
        for old in crate::phase_commands("P05") {
            assert!(crate::phase_commands("P06").contains(&old));
        }
    }

    #[test]
    fn p07_critic_requires_exact_recovery_sources_phase_and_report() {
        let inputs = serde_json::json!([
            {"path":"tools/xtask/src/persistence/session.rs","sha256":"auditor"},
            {"path":"profiles/hardware/qemu-persistence-x64.json","sha256":"profile"},
            {"path":"specs/contracts/persistent-namespace-v1.md","sha256":"catalog"}
        ]);
        let report =
            serde_json::json!({"certified_phase":"P07","source_snapshot":{"files":inputs}});
        let good = serde_json::json!({"outcome":"passed","unresolved_findings":0,"agent":"critic","phase":"P07","evidence_report":"reports/run.json","evidence_report_sha256":"exact","reviewed_build_inputs":inputs});
        assert!(validate_critic(&good, &report, "reports/run.json", "exact").is_ok());
        for (field, value) in [
            ("phase", serde_json::json!("P06")),
            ("evidence_report_sha256", Value::Null),
            ("reviewed_build_inputs", serde_json::json!([])),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert!(validate_critic(&bad, &report, "reports/run.json", "exact").is_err());
        }
        for old in crate::phase_commands("P06") {
            assert!(crate::phase_commands("P07").contains(&old));
        }
    }

    #[test]
    fn p08_critic_binds_service_sources_and_preserves_prior_required_commands() {
        let inputs = serde_json::json!([
            {"path":"profiles/hardware/qemu-services-x64.json","sha256":"profile"},
            {"path":"services/src/init.rs","sha256":"supervisor"},
            {"path":"tests/fixtures/native-workload/linker.ld","sha256":"independent linker"},
            {"path":"specs/abi/native-platform-v1.md","sha256":"platform contract"}
        ]);
        let report =
            serde_json::json!({"certified_phase":"P08","source_snapshot":{"files":inputs}});
        let good = serde_json::json!({"outcome":"passed","unresolved_findings":0,"agent":"critic","phase":"P08","evidence_report":"reports/run.json","evidence_report_sha256":"exact","reviewed_build_inputs":inputs});
        assert!(validate_critic(&good, &report, "reports/run.json", "exact").is_ok());
        for (field, value) in [
            ("phase", serde_json::json!("P07")),
            ("evidence_report_sha256", Value::Null),
            ("reviewed_build_inputs", serde_json::json!([])),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert!(validate_critic(&bad, &report, "reports/run.json", "exact").is_err());
        }
        let current = crate::phase_commands("P08");
        for old in crate::phase_commands("P07") {
            assert!(current.contains(&old));
        }
        for package in [
            "door-storage-core",
            "door-service-protocol",
            "door-service-client",
            "door-system-services",
            "door-native-workload",
        ] {
            assert!(
                current.iter().any(|command| command.contains(&package)
                    && command.contains(&"x86_64-unknown-none"))
            );
        }
    }

    #[test]
    fn p14_mandatory_commands_extend_p13() {
        let p13 = crate::phase_commands("P13");
        let p14 = crate::phase_commands("P14");
        for old in &p13 {
            assert!(p14.contains(old));
        }
        assert!(p14.iter().any(|argv| argv.contains(&"door-desktop-shell")
            && argv.contains(&"clippy")
            && argv.contains(&"x86_64-unknown-none")));
        for package in [
            "door-design-tokens",
            "door-file-ops",
            "door-settings-registry",
            "door-settings-map",
        ] {
            assert!(!p13.iter().any(|argv| argv.contains(&package)));
            assert!(p14.iter().any(|argv| argv.contains(&package)
                && argv.contains(&"clippy")
                && argv.contains(&"x86_64-unknown-none")));
            assert!(
                p14.iter()
                    .any(|argv| argv.contains(&package) && argv.contains(&"test"))
            );
        }
        for filter in ["entropy", "credentials", "login"] {
            assert!(
                p14.iter()
                    .any(|argv| argv.contains(&"--lib") && argv.contains(&filter))
            );
        }
        assert_eq!(crate::phase_commands("P18"), crate::gate_commands());
    }

    #[test]
    fn mandatory_commands_never_repeat_an_exact_invocation() {
        for phase in ["P11", "P12", "P13", "P14"] {
            let commands = crate::phase_commands(phase);
            let mut unique = commands.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), commands.len(), "{phase} repeats a command");
        }
    }

    #[test]
    fn p12_and_p13_mandatory_commands_are_cumulative() {
        let p11 = crate::phase_commands("P11");
        let p12 = crate::phase_commands("P12");
        let p13 = crate::phase_commands("P13");
        for old in &p11 {
            assert!(p12.contains(old));
        }
        for old in &p12 {
            assert!(p13.contains(old));
        }
        assert!(p12.len() > p11.len());
        for package in ["door-app-ui", "door-preview", "door-monitor"] {
            assert!(p12.iter().any(|argv| argv.contains(&package)
                && argv.contains(&"clippy")
                && argv.contains(&"x86_64-unknown-none")));
        }
        for package in [
            "door-ui",
            "door-ui-native",
            "door-accessibility",
            "door-keyboard-layouts",
            "door-locale",
            "door-gallery",
            "door-ime",
        ] {
            assert!(p12.iter().any(|argv| argv.contains(&package)
                && argv.contains(&"clippy")
                && argv.contains(&"x86_64-unknown-none")));
            assert!(!p11.iter().any(|argv| argv.contains(&package)));
        }
        for package in [
            "door-ui",
            "door-accessibility",
            "door-gallery",
            "door-graphics-text",
            "door-keyboard-layouts",
            "door-ime",
        ] {
            assert!(
                p12.iter()
                    .any(|argv| argv[..2] == ["cargo", "test"] && argv.contains(&package))
            );
        }
        assert!(p12.contains(&vec![
            "cargo",
            "test",
            "--locked",
            "--package",
            "door-ime",
            "--test",
            "corpus"
        ]));
        for filter in ["text_input", "clipboard", "dnd"] {
            assert!(
                p12.iter()
                    .any(|argv| argv.contains(&"door-window") && argv.last() == Some(&filter))
            );
        }
        // An unknown phase never inherits a later gate's commands.
        assert_eq!(crate::phase_commands("P18"), crate::gate_commands());
    }

    #[test]
    fn acquired_preflight_requires_every_successful_command() {
        let commands: Vec<_> = [vec!["--version"], vec!["-machine", "help"], vec!["-cpu", "help"], vec!["-accel", "help"]]
            .into_iter().map(|args| {
                let argv: Vec<_> = core::iter::once("qemu.exe").chain(args).collect();
                serde_json::json!({"argv":argv,"required":true,"outcome":"passed","exit_code":0,"expected_exit_code":0,"timed_out":false,"output_limit_exceeded":false,"spawn_error":null,"cleanup_error":null})
            }).collect();
        assert!(validate_preflight_commands(&commands, "qemu.exe").is_ok());
        for index in 0..commands.len() {
            let mut missing = commands.clone();
            missing.remove(index);
            assert!(validate_preflight_commands(&missing, "qemu.exe").is_err());
            let mut failed = commands.clone();
            failed[index]["exit_code"] = serde_json::json!(1);
            assert!(validate_preflight_commands(&failed, "qemu.exe").is_err());
        }
    }

    #[test]
    fn a_single_build_cannot_be_duplicated_as_independent_rebuild_evidence() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let duplicate = serde_json::json!({"path":"out/builds/one/pass-0/x86_64-pc-windows-msvc/release/door-xtask.exe"});
        assert!(
            validate_rebuild_identity(root, &[duplicate.clone(), duplicate], &[], &Value::Null)
                .is_err()
        );
        assert!(!successful_command(
            &serde_json::json!({"required":true,"outcome":"passed","exit_code":101,"timed_out":false,"output_limit_exceeded":false})
        ));
    }

    const RUN: &str = "host-1791824568839707600-11968";
    const DIGEST: &str = "bf216d9a984caea4a49beb64d7e58d035c64f31c5865f170bd529571968cc3d1";

    /// A private repository holding only the retained critic and the ADR; the
    /// qualifying report itself is absent, as after the owner's cleanup.
    fn historical_fixture() -> (crate::test_support::Workspace, Value, Value) {
        let workspace = crate::test_support::Workspace::new();
        let root = &workspace.root;
        fs::create_dir_all(root.join("reports/critics")).unwrap();
        fs::create_dir_all(root.join("docs/adr")).unwrap();
        fs::write(
            root.join(DISPOSITION_RECORD),
            "# Historical evidence disposition\n",
        )
        .unwrap();
        let evidence = format!("reports/runs/{RUN}/report.json");
        let critic = serde_json::json!({"phase":"P05","agent":"/root/p05_critic","outcome":"passed",
            "unresolved_findings":0,"evidence_report":evidence,"evidence_report_sha256":DIGEST,
            "reviewed_build_inputs":[{"path":"kernel/src/lib.rs","sha256":"reviewed"}]});
        fs::write(
            root.join("reports/critics/p05-critic.json"),
            serde_json::to_vec(&critic).unwrap(),
        )
        .unwrap();
        let phase = serde_json::json!({"id":"P05","phase_status":"passed",
            "evidence_report":evidence,"critic_review":"reports/critics/p05-critic.json",
            "evidence_disposition":{"state":"unavailable_owner_confirmed","date":"2026-10-23",
                "owner_confirmed":true,
                "owner_statement":"P00-P11 were completed; their reports were deleted by the owner for disk space.",
                "original_run_id":RUN,"original_report_sha256":DIGEST,
                "decision_record":DISPOSITION_RECORD}});
        let ledger = serde_json::json!({"active_phase":"P12"});
        (workspace, ledger, phase)
    }

    #[test]
    fn owner_confirmed_disposition_accepts_a_missing_historical_report_without_recreating_it() {
        let (workspace, ledger, phase) = historical_fixture();
        let root = &workspace.root;
        assert_eq!(
            validate_passed_phase(root, &ledger, &phase),
            Ok(Evidence::OwnerConfirmedUnavailable)
        );
        assert!(!root.join(format!("reports/runs/{RUN}")).exists());
        assert_eq!(fs::read_dir(root.join("reports")).unwrap().count(), 1);
        // The critic file is still required historical identity.
        fs::remove_file(root.join("reports/critics/p05-critic.json")).unwrap();
        assert!(validate_passed_phase(root, &ledger, &phase).is_err());
    }

    /// ADR 0027: P12-P14 passed on the owner's word, with no report or critic.
    #[test]
    fn an_owner_attested_pass_is_bounded_to_p12_p14_and_claims_no_evidence() {
        let workspace = crate::test_support::Workspace::new();
        let root = &workspace.root;
        fs::create_dir_all(root.join("docs/adr")).unwrap();
        fs::write(root.join(ATTESTED_RECORD), "# P12-P14 attested\n").unwrap();
        let attested = |id: &str| {
            serde_json::json!({"id":id,"phase_status":"passed",
                "evidence_report":null,"critic_review":null,
                "evidence_disposition":{"state":ATTESTED_STATE,"date":ATTESTED_DATE,
                    "owner_confirmed":true,
                    "owner_statement":"p12-14 passed but i deleted evidence before you saw it",
                    "decision_record":ATTESTED_RECORD}})
        };
        let ledger = serde_json::json!({"active_phase":"P15"});
        for id in ["P12", "P13", "P14"] {
            assert_eq!(
                validate_passed_phase(root, &ledger, &attested(id)),
                Ok(Evidence::OwnerAttested)
            );
        }
        // Never another phase, never the selected one.
        assert!(validate_passed_phase(root, &ledger, &attested("P15")).is_err());
        assert!(validate_passed_phase(root, &ledger, &attested("P11")).is_err());
        let selected = serde_json::json!({"active_phase":"P13"});
        assert!(validate_passed_phase(root, &selected, &attested("P13")).is_err());
        // No report or critic may be claimed, and the record must be complete.
        let mut claimed = attested("P12");
        claimed["evidence_report"] = serde_json::json!("reports/runs/x/report.json");
        assert!(validate_passed_phase(root, &ledger, &claimed).is_err());
        let mut critic = attested("P12");
        critic["critic_review"] = serde_json::json!("reports/critics/p12.json");
        assert!(validate_passed_phase(root, &ledger, &critic).is_err());
        let mut undated = attested("P12");
        undated["evidence_disposition"]["date"] = serde_json::json!("2026-10-23");
        assert!(validate_passed_phase(root, &ledger, &undated).is_err());
        fs::remove_file(root.join(ATTESTED_RECORD)).unwrap();
        assert!(validate_passed_phase(root, &ledger, &attested("P12")).is_err());
    }

    #[test]
    fn a_phase_without_the_disposition_still_needs_its_report() {
        let (workspace, ledger, mut phase) = historical_fixture();
        phase
            .as_object_mut()
            .unwrap()
            .remove("evidence_disposition");
        let error = validate_passed_phase(&workspace.root, &ledger, &phase).unwrap_err();
        assert!(
            error.contains("no reviewed evidence_disposition"),
            "{error}"
        );
        // A present report is fully revalidated even with a disposition: an
        // unqualified report cannot hide behind the owner record.
        let (workspace, ledger, phase) = historical_fixture();
        let directory = workspace.root.join(format!("reports/runs/{RUN}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("report.json"), b"{\"success\":false}").unwrap();
        assert_eq!(
            validate_passed_phase(&workspace.root, &ledger, &phase),
            Err("passed phase requires a successful full verify run".into())
        );
        // Traversal is rejected before existence is considered, and can never be
        // treated as an owner-confirmed unavailable report.
        let (workspace, ledger, mut phase) = historical_fixture();
        phase["evidence_report"] = serde_json::json!(format!("../{RUN}/report.json"));
        assert!(validate_passed_phase(&workspace.root, &ledger, &phase).is_err());
        phase
            .as_object_mut()
            .unwrap()
            .remove("evidence_disposition");
        assert!(
            validate_passed_phase(&workspace.root, &ledger, &phase)
                .unwrap_err()
                .contains("invalid relative path")
        );
    }

    #[test]
    fn a_forged_or_incomplete_disposition_fails() {
        let (workspace, ledger, phase) = historical_fixture();
        let root = &workspace.root;
        for field in [
            "state",
            "owner_confirmed",
            "owner_statement",
            "date",
            "original_run_id",
            "decision_record",
            "original_report_sha256",
        ] {
            let mut forged = phase.clone();
            forged["evidence_disposition"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(
                validate_passed_phase(root, &ledger, &forged).is_err(),
                "{field}"
            );
        }
        for (field, value) in [
            ("state", serde_json::json!("unavailable")),
            ("owner_confirmed", serde_json::json!(false)),
            ("owner_confirmed", serde_json::json!("true")),
            ("owner_statement", serde_json::json!(" ")),
            ("date", serde_json::json!("2026-10-24")),
            ("original_run_id", serde_json::json!("host-1-2")),
            (
                "decision_record",
                serde_json::json!("docs/adr/0001-project-baseline.md"),
            ),
            (
                "original_report_sha256",
                serde_json::json!(DIGEST.replace('b', "c")),
            ),
        ] {
            let mut forged = phase.clone();
            forged["evidence_disposition"][field] = value;
            assert!(
                validate_passed_phase(root, &ledger, &forged).is_err(),
                "{field}"
            );
        }
        // The selected phase and phases beyond the owner's approval never qualify.
        let current = serde_json::json!({"active_phase":"P05"});
        assert!(
            validate_passed_phase(root, &current, &phase)
                .unwrap_err()
                .contains("selected phase")
        );
        let mut later = phase.clone();
        later["id"] = serde_json::json!("P12");
        assert!(
            validate_passed_phase(root, &serde_json::json!({"active_phase":"P13"}), &later)
                .unwrap_err()
                .contains("P00-P11")
        );
        // A critic that reviewed another run or failed cannot anchor the identity.
        for (field, value) in [
            (
                "evidence_report",
                serde_json::json!("reports/runs/host-1-2/report.json"),
            ),
            ("outcome", serde_json::json!("failed")),
            ("unresolved_findings", serde_json::json!(1)),
            ("phase", serde_json::json!("P04")),
            ("reviewed_build_inputs", serde_json::json!([])),
        ] {
            let (workspace, ledger, phase) = historical_fixture();
            let path = workspace.root.join("reports/critics/p05-critic.json");
            let mut critic: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            critic[field] = value;
            fs::write(&path, serde_json::to_vec(&critic).unwrap()).unwrap();
            assert!(
                validate_passed_phase(&workspace.root, &ledger, &phase).is_err(),
                "{field}"
            );
        }
        // Without the reviewed ADR the disposition has no decision record.
        let (workspace, ledger, phase) = historical_fixture();
        fs::remove_file(workspace.root.join(DISPOSITION_RECORD)).unwrap();
        assert!(validate_passed_phase(&workspace.root, &ledger, &phase).is_err());
    }

    fn parallel_ledger() -> Value {
        let mut phases: Vec<_> = (0..16)
            .map(|index| serde_json::json!({"id":format!("P{index:02}"),"phase_status":"passed"}))
            .collect();
        phases[12]["phase_status"] = serde_json::json!("in_progress");
        phases[13] = serde_json::json!({"id":"P13","phase_status":"in_progress",
            "parallel_authorization":{"with":"P12","owner_confirmed":true,"date":"2026-10-23",
                "owner_decision":"Owner decision 6: P12 and P13 proceed together.",
                "decision_record":"docs/design/p12-p13-plan-20261023.md"}});
        for phase in &mut phases[14..] {
            phase["phase_status"] = serde_json::json!("not_started");
        }
        serde_json::json!({"active_phase":"P12","phases":phases})
    }

    #[test]
    fn owner_authorized_parallel_phase_is_explicit_contiguous_and_cannot_pass_first() {
        let ledger = parallel_ledger();
        assert!(validate_active_phase(&ledger).is_ok());
        assert_eq!(active_phases(&ledger).unwrap(), ["P12", "P13"]);
        assert_eq!(verification_phase(&ledger, None).unwrap(), "P12");
        assert_eq!(verification_phase(&ledger, Some("P12")).unwrap(), "P12");
        assert_eq!(verification_phase(&ledger, Some("P13")).unwrap(), "P13");
        for other in ["P11", "P14", "P99", ""] {
            assert!(verification_phase(&ledger, Some(other)).is_err(), "{other}");
        }
        let mut blocked = ledger.clone();
        blocked["phases"][13]["phase_status"] = serde_json::json!("blocked");
        assert!(validate_active_phase(&blocked).is_ok());
        for state in ["passed", "regression"] {
            let mut bad = ledger.clone();
            bad["phases"][13]["phase_status"] = serde_json::json!(state);
            assert!(validate_active_phase(&bad).is_err(), "{state}");
        }
        for field in [
            "with",
            "owner_confirmed",
            "date",
            "owner_decision",
            "decision_record",
        ] {
            let mut bad = ledger.clone();
            bad["phases"][13]["parallel_authorization"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(validate_active_phase(&bad).is_err(), "{field}");
        }
        for (field, value) in [
            ("with", serde_json::json!("P11")),
            ("owner_confirmed", serde_json::json!(false)),
            ("date", serde_json::json!("2026-13-01")),
            ("date", serde_json::json!("23 October")),
        ] {
            let mut bad = ledger.clone();
            bad["phases"][13]["parallel_authorization"][field] = value;
            assert!(validate_active_phase(&bad).is_err(), "{field}");
        }
        // No gaps, and an unimplemented schema cannot be authorized.
        let mut gap = ledger.clone();
        gap["phases"][14] = gap["phases"][13].clone();
        gap["phases"][14]["id"] = serde_json::json!("P14");
        gap["phases"][13] = serde_json::json!({"id":"P13","phase_status":"not_started"});
        assert!(validate_active_phase(&gap).is_err());
        // P14 is an explicit schema and may be authorized after P13; P15 is not.
        let mut authorized = ledger.clone();
        authorized["phases"][14] = authorized["phases"][13].clone();
        authorized["phases"][14]["id"] = serde_json::json!("P14");
        assert!(validate_active_phase(&authorized).is_ok());
        assert_eq!(active_phases(&authorized).unwrap(), ["P12", "P13", "P14"]);
        assert_eq!(verification_phase(&authorized, Some("P14")).unwrap(), "P14");
        // P15, P16 and P17 have schemas and may join the chain; P18 has none.
        let mut chain = authorized.clone();
        chain["phases"][15] = chain["phases"][13].clone();
        chain["phases"][15]["id"] = serde_json::json!("P15");
        let mut p16 = chain["phases"][13].clone();
        p16["id"] = serde_json::json!("P16");
        chain["phases"].as_array_mut().unwrap().push(p16);
        let mut p17 = chain["phases"][13].clone();
        p17["id"] = serde_json::json!("P17");
        chain["phases"].as_array_mut().unwrap().push(p17);
        assert!(validate_active_phase(&chain).is_ok());
        let mut unimplemented = chain.clone();
        let mut p18 = unimplemented["phases"][13].clone();
        p18["id"] = serde_json::json!("P18");
        unimplemented["phases"].as_array_mut().unwrap().push(p18);
        assert!(validate_active_phase(&unimplemented).is_err());
        // Without the authorization P13 is an ordinary premature phase.
        let mut plain = ledger;
        plain["phases"][13]
            .as_object_mut()
            .unwrap()
            .remove("parallel_authorization");
        assert!(validate_active_phase(&plain).is_err());
        assert!(verification_phase(&plain, Some("P13")).is_err());
    }

    #[test]
    fn repository_ledger_records_reviewed_dispositions_and_parallel_phases() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let ledger: Value =
            serde_json::from_slice(&fs::read(root.join("project/phases.json")).unwrap()).unwrap();
        validate_active_phase(&ledger).unwrap();
        let active = active_phases(&ledger).unwrap();
        for phase in ledger["phases"].as_array().unwrap() {
            let current = active.iter().any(|id| phase["id"] == id.as_str());
            if phase["evidence_disposition"]["state"] == ATTESTED_STATE {
                validate_attestation(&root, phase, current).unwrap();
                assert_eq!(
                    validate_passed_phase(&root, &ledger, phase),
                    Ok(Evidence::OwnerAttested),
                    "{}",
                    phase["id"]
                );
                continue;
            }
            if !phase["evidence_disposition"].is_null() {
                validate_disposition(&root, phase, current).unwrap();
            }
            // Every passed phase whose report is gone must carry the reviewed
            // disposition; spec::check then accepts it without reopening it.
            if phase["phase_status"] == "passed"
                && !evidence_present(&root, phase["evidence_report"].as_str().unwrap()).unwrap()
            {
                assert_eq!(
                    validate_passed_phase(&root, &ledger, phase),
                    Ok(Evidence::OwnerConfirmedUnavailable),
                    "{}",
                    phase["id"]
                );
            }
            if !phase["parallel_authorization"].is_null() {
                checked_file(
                    &root,
                    phase["parallel_authorization"]["decision_record"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
            }
        }
    }
}
