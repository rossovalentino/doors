//! Drives the real service-backed console through QMP keyboard events. The
//! interactive instance receives a private copy of the saved qualification disk.
use super::{
    PROFILE, audit,
    scenarios::{self, Mode},
};
use crate::{
    emulator,
    evidence::{Run, file_hash, relative, write_json},
    paths, qmp,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    net::TcpListener,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn commands(checked_rename: bool) -> &'static [&'static str] {
    if checked_rename {
        &scenarios::RENAME_COMMANDS
    } else {
        &scenarios::CONSOLE_COMMANDS
    }
}
fn validate_serial(serial: &str, checked_rename: bool) -> bool {
    if checked_rename {
        scenarios::validate_rename_console(serial).is_some()
    } else {
        scenarios::validate(serial, Mode::Demo).is_some()
    }
}
fn native_path(path: &Path) -> Result<String, String> {
    Ok(path
        .to_str()
        .ok_or("console path")?
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/"))
}
pub(super) fn arguments(
    mut args: Vec<String>,
    disk: &Path,
    profile: &Value,
    visible: bool,
    port: u16,
    uuid: &str,
) -> Result<Vec<String>, String> {
    if port == 0 || uuid.len() != 36 || !uuid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err("console control identity".into());
    }
    let at = args
        .iter()
        .position(|arg| arg == "-display")
        .ok_or("console display argument")?;
    *args.get_mut(at + 1).ok_or("console display value")? =
        if visible { "sdl" } else { "none" }.into();
    args.extend(emulator::storage_arguments(profile, disk)?);
    args.extend([
        "-name".into(),
        "Door OS - native service console".into(),
        "-uuid".into(),
        uuid.into(),
        "-qmp".into(),
        format!("tcp:127.0.0.1:{port},server=on,wait=off"),
        "-fw_cfg".into(),
        "name=opt/door/mode,string=services-demo".into(),
    ]);
    Ok(args)
}
pub(super) fn wait_marker(
    child: &mut crate::evidence::ControlledProcess<'_>,
    serial: &Path,
    marker: &str,
    seconds: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let captured = fs::read_to_string(serial).unwrap_or_default();
        if captured
            .lines()
            .any(|line| line.starts_with("DOOR_USER pid=") && line.ends_with(marker))
        {
            return Ok(());
        }
        if captured.contains("KERNEL_PANIC")
            || captured.contains("DOOR_KERNEL FAILURE")
            || captured.contains("DOOR_P08 USER_FAULT")
            || captured.contains("DOOR_P08 INIT FAIL")
        {
            return Err("service console guest failure".into());
        }
        if child.poll()?.is_some() {
            return Err("service console exited before expected operation".into());
        }
        if Instant::now() >= deadline {
            return Err(format!("service console deadline: {marker}"));
        }
        thread::sleep(Duration::from_millis(20));
    }
}
fn screen(client: &mut qmp::Client, root: &Path, path: &Path) -> Result<Value, String> {
    client.execute(
        "screendump",
        json!({"filename":native_path(path)?,"format":"png"}),
    )?;
    screen_binding(root, path)
}
fn screen_binding(root: &Path, path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err("service console screenshot is not a PNG".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width < 640 || height < 480 || width > 4096 || height > 4096 {
        return Err("service console screenshot dimensions".into());
    }
    Ok(json!({"path":relative(root,path)?,"sha256":file_hash(path)?,"width":width,"height":height}))
}
pub fn launch(
    run: &mut Run,
    image: &str,
    baseline: &str,
    visible: bool,
    exercise: bool,
) -> Result<Value, String> {
    launch_with(run, image, baseline, visible, exercise, false)
}
fn launch_with(
    run: &mut Run,
    image: &str,
    baseline: &str,
    visible: bool,
    exercise: bool,
    checked_rename: bool,
) -> Result<Value, String> {
    let saved = paths::existing(&run.root, baseline)?;
    let before = audit::inspect(&saved)?;
    if before["catalog"]["state"] != "saved" {
        return Err("service console requires saved SDK workload".into());
    }
    let disk_dir = paths::fresh_directory(&run.root, &format!("out/storage/{}-p08-demo", run.id))?;
    for name in ["initial.raw", "data.raw"] {
        fs::copy(&saved, disk_dir.join(name)).map_err(|e| e.to_string())?;
    }
    let data = disk_dir.join("data.raw");
    let prepared = emulator::prepare(run, PROFILE, image)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    let hash = format!("{:x}", Sha256::digest(run.id.as_bytes()));
    let uuid = format!(
        "{}-{}-{}-{}-{}",
        &hash[..8],
        &hash[8..12],
        &hash[12..16],
        &hash[16..20],
        &hash[20..32]
    );
    let args = arguments(
        prepared.args,
        &data,
        &prepared.profile,
        visible,
        port,
        &uuid,
    )?;
    let root = run.root.clone();
    let token = run.id.clone();
    let mut record=run.controlled_expected(prepared.qemu.to_str().ok_or("QEMU path")?,&args,&[(&prepared.serial,1024*1024)],33,|child|{
        wait_marker(child,&prepared.serial,"DOOR_P08 CONSOLE READY",30)?;
        let mut client=qmp::Client::connect(port)?;
        if client.execute("query-uuid",json!({}))?["UUID"]!=uuid {return Err("service console QMP identity differs".into());}
        let mut screens=vec![screen(&mut client,&root,&prepared.private.join("console-ready.png"))?];
        let mut interactions=Vec::new();
        if exercise {
            for &command in commands(checked_rename) {
                for key in command.chars().map(|c|c.to_string()).chain(std::iter::once("ret".into())) {
                    client.execute("send-key",json!({"keys":[{"type":"qcode","data":key}],"hold-time":20}))?;
                    thread::sleep(Duration::from_millis(35));
                }
                let marker=format!("DOOR_P08 CONSOLE COMMAND {command}");
                wait_marker(child,&prepared.serial,&marker,8)?;
                if command!="exit" {screens.push(screen(&mut client,&root,&prepared.private.join(format!("console-{command}.png")))?);}
                interactions.push(json!({"command":command,"marker":marker}));
            }
            if child.wait()?.code()!=Some(33){return Err("service console did not retire through expected debug exit".into());}
            let serial=fs::read_to_string(&prepared.serial).map_err(|e|e.to_string())?;
            if !validate_serial(&serial,checked_rename) || emulator::classify(Some(33),false,false,false,false,&serial,&token)!=emulator::Outcome::Passed {return Err("service console authentication/retirement evidence incomplete".into());}
        } else { child.handoff()?; }
        Ok(json!({"schema_version":1,"record_kind":"service_console_session","state":if exercise{"completed"}else{"running"},"visible":visible,"pid":child.id(),"qmp_port":port,"uuid":uuid,"run_token":token,"argv":args,"profile":prepared.profile,"image":prepared.image_record,
            "boot_medium":{"path":relative(&root,&prepared.private.join("esp"))?,"kind":"read-only-vvfat-via-virtio"},"private_vars":relative(&root,&prepared.vars)?,"private_vars_sha256":file_hash(&prepared.vars)?,
            "serial":{"path":relative(&root,&prepared.serial)?,"sha256":file_hash(&prepared.serial)?},"screens":screens,"interactions":interactions,
            "data_disk":{"path":relative(&root,&data)?,"initial":{"path":relative(&root,&disk_dir.join("initial.raw"))?,"sha256":file_hash(&disk_dir.join("initial.raw"))?,"audit":before}},"scope":"native CPL3 application and console service"}))
    })?;
    crate::image::validate_staged(
        &root,
        &relative(&root, &prepared.private.join("image.json"))?,
    )?;
    emulator::acquired_inputs(&prepared.profile)?;
    if exercise {
        let snapshot = prepared.private.join("data-after.raw");
        fs::copy(&data, &snapshot).map_err(|e| e.to_string())?;
        let after = if checked_rename {
            audit::validate_rename_change(&disk_dir.join("initial.raw"), &snapshot)?;
            audit::inspect_rename(&snapshot)?
        } else {
            audit::validate_demo_change(&disk_dir.join("initial.raw"), &snapshot)?;
            audit::inspect_demo(&snapshot)?
        };
        record["data_disk"]["after"] =
            json!({"path":relative(&root,&snapshot)?,"sha256":file_hash(&snapshot)?,"audit":after});
    }
    record["process_command"] =
        run.commands().last().ok_or("service console command")?["id"].clone();
    record["exercise"] = json!(if checked_rename {
        "checked_rename"
    } else {
        "standard"
    });
    if file_hash(&saved)? != before["sha256"] {
        return Err("service console changed its source baseline".into());
    }
    write_json(&run.directory.join("service-console.json"), &record)?;
    if !exercise {
        write_json(&root.join("out/demo-session.json"), &record)?;
    }
    Ok(record)
}

pub fn validate(root: &Path, parent: &Value, suite: &Value, staged: &Value) -> Result<(), String> {
    validate_with(root, parent, suite, staged, false)
}
fn validate_with(
    root: &Path,
    parent: &Value,
    suite: &Value,
    staged: &Value,
    checked_rename: bool,
) -> Result<(), String> {
    let binding = &suite["service_demo"];
    let record = crate::boot_gate::checked_record(root, binding)?;
    let child = crate::boot_gate::checked_record(root, &binding["report"])?;
    let expected_exercise = if checked_rename {
        "checked_rename"
    } else {
        "standard"
    };
    // The field is optional only for retained historical standard-console runs.
    if record
        .get("exercise")
        .is_some_and(|value| value != expected_exercise)
        || checked_rename && record["exercise"] != expected_exercise
    {
        return Err("service console exercise variant differs".into());
    }
    crate::boot_gate::logs(root, &child, Some((&record["process_command"], 33)))?;
    let token = child["id"].as_str().ok_or("console report token")?;
    if record["state"] != "completed"
        || record["visible"] != false
        || record["image"] != *staged
        || child["success"] != true
        || child["source_snapshot"] != parent["source_snapshot"]
        || record["run_token"] != token
    {
        return Err("service console report/source binding differs".into());
    }
    let base = crate::boot_gate::private_launch(root, &record, &child, staged, PROFILE)?;
    let disk = format!("out/storage/{token}-p08-demo/data.raw");
    if record["data_disk"]["path"] != disk {
        return Err("service console disk does not belong to run".into());
    }
    let port = u16::try_from(record["qmp_port"].as_u64().ok_or("console port")?)
        .map_err(|e| e.to_string())?;
    let args = arguments(
        base,
        &crate::storage::confined(root, &disk)?,
        &record["profile"],
        false,
        port,
        record["uuid"].as_str().ok_or("console UUID")?,
    )?;
    if record["argv"] != json!(args) {
        return Err("service console launch arguments changed".into());
    }
    let process = crate::boot_gate::process(&child, &record["profile"], &args)?;
    if process["required"] != true
        || process["outcome"] != "passed"
        || process["expected_exit_code"] != 33
        || process["exit_code"] != 33
        || process["timeout_ms"] != 90000
        || process["timed_out"] != false
        || !process["spawn_error"].is_null()
        || !process["controller_error"].is_null()
        || !process["cleanup_error"].is_null()
        || process["output_limit_exceeded"] != false
        || process["lifecycle"] != "completed"
        || process["id"] != record["process_command"]
    {
        return Err("service console process did not complete under its bounded policy".into());
    }
    let serial = paths::existing(
        root,
        record["serial"]["path"].as_str().ok_or("console serial")?,
    )?;
    if file_hash(&serial)? != record["serial"]["sha256"] {
        return Err("service console serial changed".into());
    }
    let text = fs::read_to_string(serial).map_err(|e| e.to_string())?;
    if !validate_serial(&text, checked_rename)
        || emulator::classify(Some(33), false, false, false, false, &text, token)
            != emulator::Outcome::Passed
    {
        return Err("service console actual effects incomplete".into());
    }
    // Historical rename records predate instrumentation. An included sample
    // must always be recomputed from the sealed, authenticated native serial.
    if let Some(sample) = suite.get("rpc_latency")
        && (!checked_rename || !scenarios::rpc_latency_matches(&text, sample))
    {
        return Err("SDK RPC measurement differs from authenticated serial".into());
    }
    let expected:Vec<_>=commands(checked_rename).iter().map(|command|json!({"command":command,"marker":format!("DOOR_P08 CONSOLE COMMAND {command}")})).collect();
    if record["interactions"] != json!(expected) {
        return Err("service console interaction inventory changed".into());
    }
    let screens = record["screens"]
        .as_array()
        .ok_or("service console screens")?;
    let mut hashes = std::collections::BTreeSet::new();
    for (index, name) in [
        "ready",
        "help",
        "status",
        "file",
        if checked_rename { "rename" } else { "theme" },
        "threads",
    ]
    .into_iter()
    .enumerate()
    {
        let path = format!("out/qemu/{token}/console-{name}.png");
        let screen = screen_binding(root, &paths::existing(root, &path)?)?;
        if screens.get(index) != Some(&screen)
            || !hashes.insert(screen["sha256"].as_str().unwrap().to_owned())
        {
            return Err("service console screen evidence incomplete or unchanged".into());
        }
    }
    if screens.len() != 6 {
        return Err("service console needs all six actual screens".into());
    }
    let initial = format!("out/storage/{token}-p08-demo/initial.raw");
    let after = format!("out/qemu/{token}/data-after.raw");
    if record["data_disk"]["initial"]["path"] != initial
        || record["data_disk"]["after"]["path"] != after
        || record["data_disk"]["initial"]["sha256"] != suite["services"]["saved"]["sha256"]
    {
        return Err("service console did not clone saved suite state".into());
    }
    let initial = paths::existing(root, &initial)?;
    let after = paths::existing(root, &after)?;
    if audit::inspect(&initial)? != record["data_disk"]["initial"]["audit"]
        || file_hash(&initial)? != record["data_disk"]["initial"]["sha256"]
        || (if checked_rename {
            audit::inspect_rename(&after)?
        } else {
            audit::inspect_demo(&after)?
        }) != record["data_disk"]["after"]["audit"]
        || file_hash(&after)? != record["data_disk"]["after"]["sha256"]
        || file_hash(&paths::existing(root, &disk)?)? != record["data_disk"]["after"]["sha256"]
    {
        return Err("service console persisted effect changed".into());
    }
    if checked_rename {
        audit::validate_rename_change(&initial, &after)
    } else {
        audit::validate_demo_change(&initial, &after)
    }
}

/// An explicit SDK exercise on a private copy; the original service suite and
/// its read-only reopen/restart assertions keep their unchanged contracts.
pub fn checked_rename_test(run: &mut Run, image: &str, baseline: &str) -> Result<(), String> {
    let saved = paths::existing(&run.root, baseline)?;
    let baseline_hash = file_hash(&saved)?;
    let mut child = Run::new(run.root.clone())?;
    let result = launch_with(&mut child, image, baseline, false, true, true);
    child.finish(result.as_ref().err().map(String::as_str))?;
    result?;
    let console_record = crate::image::read_json(&child.directory.join("service-console.json"))?;
    let measured_serial = fs::read_to_string(paths::existing(
        &run.root,
        console_record["serial"]["path"]
            .as_str()
            .ok_or("SDK measured serial path")?,
    )?)
    .map_err(|e| e.to_string())?;
    let rpc_sample =
        scenarios::rpc_latency_record(&measured_serial).ok_or("SDK RPC measurement attribution")?;
    let suite = json!({
        "schema_version": 1,
        "record_kind": "sdk_checked_rename_suite",
        "outcome": "Passed",
        "verification_state": "guest_tested",
        "image": image,
        "baseline": { "path": baseline, "sha256": baseline_hash },
        "rpc_latency": rpc_sample,
        "services": { "saved": { "sha256": baseline_hash } },
        "service_demo": {
            "path": relative(&run.root, &child.directory.join("service-console.json"))?,
            "sha256": file_hash(&child.directory.join("service-console.json"))?,
            "report": {
                "path": relative(&run.root, &child.directory.join("report.json"))?,
                "sha256": file_hash(&child.directory.join("report.json"))?
            }
        },
        "scope": "one authenticated SDK identity; raw UTF16 roundtrip, exact two commits, stale/sharing/ancestry checks; no full filesystem compatibility claim"
    });
    let (_, staged) = crate::image::validate_staged(&run.root, image)?;
    validate_with(
        &run.root,
        &json!({"source_snapshot":run.build_inputs()}),
        &suite,
        &staged,
        true,
    )?;
    if file_hash(&saved)? != baseline_hash {
        return Err("SDK rename mutated original baseline".into());
    }
    write_json(&run.directory.join("sdk-checked-rename.json"), &suite)?;
    run.record_guest(&suite)?;
    run.check("p12-sdk-checked-rename", suite);
    Ok(())
}
