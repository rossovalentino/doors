//! Fast integration entry point for the P08 service slice. This retains real
//! process/disk/UI evidence but cannot substitute for the cumulative phase gate.
use super::{demo, scenarios, session::Session};
use crate::{
    emulator,
    evidence::{Run, file_hash, relative, write_json},
    image,
    phase::Phase,
};
use serde_json::json;
use std::fs;

pub fn run(run: &mut Run, image_path: &str) -> Result<(), String> {
    let ledger = image::read_json(&run.root.join("project/phases.json"))?;
    if Phase::parse(ledger["active_phase"].as_str().ok_or("active phase")?)? < Phase::P08 {
        return Err("service integration requires active P08".into());
    }
    let (_, staged) = image::validate_staged(&run.root, image_path)?;
    let mut session = Session::new(run)?;
    let mut guests = Vec::new();
    let mut results = Vec::new();
    for case in scenarios::cases() {
        let disk = session.disk(case.mode)?.to_owned();
        let mut child = Run::new(run.root.clone())?;
        let observed = emulator::scenario_with_disk(
            &mut child,
            case.profile,
            image_path,
            case.mode,
            Some(&disk),
        );
        let failure = observed.as_ref().err().map(String::as_str).or_else(|| {
            (!observed
                .as_ref()
                .is_ok_and(|outcome| *outcome == case.expected))
            .then_some("service guest did not complete")
        });
        child.finish(failure)?;
        let guest_path = child.directory.join("guest-run.json");
        let guest = image::read_json(&guest_path)?;
        let serial = fs::read_to_string(
            run.root
                .join(guest["serial"]["path"].as_str().ok_or("service serial")?),
        )
        .map_err(|e| e.to_string())?;
        if !observed.is_ok_and(|outcome| outcome == case.expected)
            || !crate::scenarios::serial_matches("P08", &case.markers, &serial)
        {
            return Err(format!(
                "{} actual process/identity/serial check failed",
                case.mode
            ));
        }
        results.push(json!({"scenario":case.id,"expected":"Passed","observed":guest["outcome"],"markers":case.markers,
            "report":{"path":relative(&run.root,&child.directory.join("report.json"))?,"sha256":file_hash(&child.directory.join("report.json"))?},
            "guest":{"path":relative(&run.root,&guest_path)?,"sha256":file_hash(&guest_path)?}}));
        session.observe(guest.clone())?;
        guests.push(guest);
    }
    let evidence = session.evidence()?;
    let saved = evidence["saved"]["path"]
        .as_str()
        .ok_or("saved SDK state")?;
    let mut interactive = Run::new(run.root.clone())?;
    let result = demo::launch(&mut interactive, image_path, saved, false, true);
    interactive.finish(result.as_ref().err().map(String::as_str))?;
    result?;
    let demo_path = interactive.directory.join("service-console.json");
    let record = json!({"schema_version":1,"record_kind":"service_integration_suite","outcome":"Passed","verification_state":"guest_tested","image":image_path,"scenarios":results,"services":evidence,
        "service_demo":{"path":relative(&run.root,&demo_path)?,"sha256":file_hash(&demo_path)?,"report":{"path":relative(&run.root,&interactive.directory.join("report.json"))?,"sha256":file_hash(&interactive.directory.join("report.json"))?}},
        "scope":"P08 integration exploration; cumulative verify and critic remain required"});
    super::session::validate(&run.root, &run.id, &record, &guests)?;
    demo::validate(
        &run.root,
        &json!({"source_snapshot":run.build_inputs()}),
        &record,
        &staged,
    )?;
    write_json(&run.directory.join("service-suite.json"), &record)?;
    run.record_guest(&record)?;
    run.check("p08-service-integration", record);
    println!("P08 service integration passed; cumulative phase qualification remains separate.");
    Ok(())
}
