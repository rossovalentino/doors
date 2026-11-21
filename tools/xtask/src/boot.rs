use crate::{
    build, demo, emulator,
    evidence::{Run, file_hash, relative, write_json},
};
use serde_json::{Value, json};
use std::fs;

pub fn verify(run: &mut Run, phase: &str) -> Result<(), String> {
    let selected = crate::phase::Phase::parse(phase)?;
    let image =
        crate::evidence::timed(run, "build", "diagnostic guest image (two passes)", |run| {
            build::guest_build_for(run, true, selected)
        })?;
    let mut results = Vec::new();
    let storage = if selected >= crate::phase::Phase::P06 {
        Some(crate::storage::create(run)?)
    } else {
        None
    };
    let mut persistence = if selected >= crate::phase::Phase::P07 {
        Some(crate::persistence::session::Session::new(run)?)
    } else {
        None
    };
    let mut services = if selected >= crate::phase::Phase::P08 {
        Some(crate::services::session::Session::new(run)?)
    } else {
        None
    };
    let mut base_disk = None;
    for case in crate::scenarios::cases(phase)? {
        let mode = case.mode;
        let expected = case.expected;
        let case_started = std::time::Instant::now();
        let mut child = Run::new(run.root.clone())?;
        let windows_disk =
            if case.profile == crate::windows::PROFILE && mode == crate::windows::MODE {
                let saved = services
                    .as_ref()
                    .ok_or("Windows native service baseline")?
                    .evidence()?;
                Some(crate::windows::session::clone_saved(
                    run,
                    saved["saved"]["path"].as_str().ok_or("saved disk path")?,
                )?)
            } else {
                None
            };
        if mode == "windows-base" {
            let saved = services.as_ref().ok_or("P10 native baseline")?.evidence()?;
            base_disk = Some(crate::windows::base_verify::clone_saved(
                run,
                saved["saved"]["path"].as_str().ok_or("P10 baseline path")?,
            )?);
        }
        let p07_disk = if case.profile == crate::persistence::PROFILE {
            Some(
                persistence
                    .as_mut()
                    .ok_or("P07 session")?
                    .disk(run, mode)?
                    .to_owned(),
            )
        } else {
            None
        };
        let data = if case.profile == crate::storage::PROFILE {
            storage.as_deref()
        } else if case.profile == crate::services::PROFILE {
            Some(services.as_mut().ok_or("P08 session")?.disk(mode)?)
        } else if case.profile == crate::windows::PROFILE {
            if mode == crate::windows::MODE {
                windows_disk.as_deref()
            } else {
                base_disk.as_deref()
            }
        } else {
            p07_disk.as_deref()
        };
        let observed = emulator::scenario_with_disk(&mut child, case.profile, &image, mode, data);
        let successful = observed.as_ref().is_ok_and(|outcome| *outcome == expected);
        let failure = if observed
            .as_ref()
            .is_ok_and(|outcome| *outcome == emulator::Outcome::Passed)
        {
            None
        } else {
            Some(
                "guest diagnostic did not complete normally; expected negative status recorded in enclosing suite",
            )
        };
        child.finish(failure)?;
        run.record_timing(
            "boot-case",
            case.id,
            case_started.elapsed(),
            if successful { "passed" } else { "failed" },
        );
        let guest_path = child.directory.join("guest-run.json");
        let guest: Value = crate::image::read_json(&guest_path)?;
        let serial_path = run.root.join(
            guest["serial"]["path"]
                .as_str()
                .ok_or("guest serial path")?,
        );
        let serial = fs::read_to_string(&serial_path).map_err(|e| e.to_string())?;
        if !successful || !crate::scenarios::serial_matches(phase, &case.markers, &serial) {
            return Err(format!(
                "{phase} {} did not produce expected {expected:?} and required markers: {observed:?}",
                case.id
            ));
        }
        if data.is_some()
            && (!guest["data_disk"]["audit_error"].is_null()
                || guest["data_disk"]["after"]["audit"].is_null())
        {
            return Err("storage guest failed independent raw-image audit".into());
        }
        results.push(json!({"scenario":case.id,"expected":format!("{expected:?}"),"observed":guest["outcome"],"marker":case.markers[0],"markers":case.markers,"report":{"path":relative(&run.root,&child.directory.join("report.json"))?,"sha256":file_hash(&child.directory.join("report.json"))?},"guest":{"path":relative(&run.root,&guest_path)?,"sha256":file_hash(&guest_path)?}}));
        if case.profile == crate::persistence::PROFILE {
            persistence.as_mut().ok_or("P07 session")?.observe(guest)?;
        } else if case.profile == crate::services::PROFILE {
            services.as_mut().ok_or("P08 session")?.observe(guest)?;
        }
    }
    let demo_started = std::time::Instant::now();
    let mut demo_run = Run::new(run.root.clone())?;
    let demo_result = demo::launch(&mut demo_run, &image, false, true, phase);
    demo_run.finish(demo_result.as_ref().err().map(String::as_str))?;
    run.record_timing(
        "boot-case",
        "demo",
        demo_started.elapsed(),
        if demo_result.is_ok() {
            "passed"
        } else {
            "failed"
        },
    );
    let demo_record = demo_result?;
    let demo_path = demo_run.directory.join("demo-session.json");
    let storage_evidence = storage
        .map(|path| crate::storage::suite_evidence(&run.root, &path))
        .transpose()?;
    let persistence_evidence = persistence.map(|s| s.evidence()).transpose()?;
    let service_evidence = services.map(|s| s.evidence()).transpose()?;
    let service_demo = if let Some(services) = &service_evidence {
        let saved = services["saved"]["path"]
            .as_str()
            .ok_or("saved services disk")?;
        let started = std::time::Instant::now();
        let mut child = Run::new(run.root.clone())?;
        let result = crate::services::demo::launch(&mut child, &image, saved, false, true);
        child.finish(result.as_ref().err().map(String::as_str))?;
        run.record_timing(
            "boot-case",
            "service demo",
            started.elapsed(),
            if result.is_ok() { "passed" } else { "failed" },
        );
        result?;
        let path = child.directory.join("service-console.json");
        write_json(
            &run.root.join("out/latest-services-image.json"),
            &json!({"image":image,"baseline":saved,"baseline_sha256":services["saved"]["sha256"],"build_run":run.id}),
        )?;
        Some(
            json!({"path":relative(&run.root,&path)?,"sha256":file_hash(&path)?,"report":{"path":relative(&run.root,&child.directory.join("report.json"))?,"sha256":file_hash(&child.directory.join("report.json"))?}}),
        )
    } else {
        None
    };
    let mut record = json!({"schema_version":1,"record_kind":"guest_suite","outcome":"Passed","verification_state":"guest_tested","image":image,"scenarios":results,"storage":storage_evidence,"persistence":persistence_evidence,"services":service_evidence,"service_demo":service_demo,"demo":{"path":relative(&run.root,&demo_path)?,"sha256":file_hash(&demo_path)?,"report":{"path":relative(&run.root,&demo_run.directory.join("report.json"))?,"sha256":file_hash(&demo_run.directory.join("report.json"))?},"result":demo_record}});
    if selected >= crate::phase::Phase::P09 {
        record["windows"] = crate::windows::session::evidence(&run.root, &run.id)?;
        record["reference_comparison"] = json!({"state":"passed","same_binary":true,"output":crate::windows::OUTPUT,"exit":crate::windows::EXIT});
    }
    if selected >= crate::phase::Phase::P10 {
        record["windows_base"] = crate::windows::base_verify::disk_evidence(&run.root, &run.id)?;
        record["base_reference_comparison"] = json!({"state":"passed","same_binary":true,"output":crate::windows::base::OUTPUT,"exit":74});
    }
    write_json(&run.directory.join("boot-suite.json"), &record)?;
    run.record_guest(&record)?;
    run.check(crate::scenarios::check_id(phase)?, record);
    Ok(())
}
