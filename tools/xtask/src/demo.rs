use crate::{
    emulator,
    evidence::{Run, file_hash, relative, write_json},
    qmp,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

fn file_path(path: &std::path::Path) -> Result<String, String> {
    Ok(path
        .to_str()
        .ok_or("demo path")?
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/"))
}
pub fn launch(
    run: &mut Run,
    image: &str,
    visible: bool,
    exercise: bool,
    phase: &str,
) -> Result<Value, String> {
    let prepared = emulator::prepare(run, "profiles/hardware/qemu-diagnostic-x64.json", image)?;
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
    let args = arguments(prepared.args, visible, port, &uuid, phase)?;
    let root = run.root.clone();
    let directory = run.directory.clone();
    let token = run.id.clone();
    let mut record = run.controlled(prepared.qemu.to_str().ok_or("qemu path")?, &args, &[(&prepared.serial, 1024 * 1024)], |child| {
        let start = Instant::now();
        let mut client = None;
        loop {
            if let Some(exit) = child.poll()? {
                return Err(format!("demo exited before readiness: {exit}"));
            }
            if start.elapsed() > Duration::from_secs(30) {
                return Err("demo readiness timeout".into());
            }
            if client.is_none() {
                client = qmp::Client::connect(port).ok();
            }
            let serial = fs::read_to_string(&prepared.serial).unwrap_or_default();
            if serial.len() > 1024 * 1024 {
                return Err("demo serial limit".into());
            }
            if serial.contains("KERNEL_PANIC")
                || serial.contains("DOOR_KERNEL FAILURE")
                || serial.contains("DOOR_LOADER v1 FAILURE")
            {
                return Err("demo failed during boot".into());
            }
            if serial.contains("DOOR_DEMO READY") && client.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let mut client = client.ok_or("QMP unavailable")?;
        if client.execute("query-uuid", json!({}))?["UUID"] != uuid {
            return Err("demo identity mismatch".into());
        }
        let screenshot = prepared.private.join("screen.png");
        client.execute(
            "screendump",
            json!({"filename":file_path(&screenshot)?,"format":"png"}),
        )?;
        let original_hash = file_hash(&screenshot)?;
        let mut interactions = Vec::new();
        if exercise {
            for (key, marker) in [("h", "DOOR_DEMO HELP"), ("d", "DOOR_DEMO DIAGNOSTICS")] {
                client.execute(
                    "send-key",
                    json!({"keys":[{"type":"qcode","data":key}],"hold-time":100}),
                )?;
                let deadline = Instant::now() + Duration::from_secs(3);
                while !fs::read_to_string(&prepared.serial)
                    .unwrap_or_default()
                    .contains(marker)
                {
                    child.poll()?;
                    if Instant::now() > deadline {
                        return Err(format!("guest did not handle {key}"));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                interactions.push(json!({"key":key,"marker":marker}));
            }
            let diagnostics = prepared.private.join("diagnostics.png");
            client.execute(
                "screendump",
                json!({"filename":file_path(&diagnostics)?,"format":"png"}),
            )?;
            if file_hash(&diagnostics)? == original_hash {
                return Err("diagnostics screen did not change".into());
            }
            let diagnostics_hash = file_hash(&diagnostics)?;
            client.execute(
                "send-key",
                json!({"keys":[{"type":"qcode","data":"r"}],"hold-time":100}),
            )?;
            let reboot_deadline = Instant::now() + Duration::from_secs(15);
            while fs::read_to_string(&prepared.serial)
                .unwrap_or_default()
                .matches("DOOR_DEMO READY")
                .count()
                < 2
            {
                child.poll()?;
                if Instant::now() > reboot_deadline {
                    return Err("diagnostic reboot did not become ready".into());
                }
                thread::sleep(Duration::from_millis(20));
            }
            interactions.push(json!({"key":"r","marker":"DOOR_DEMO REBOOT","readiness_count":2}));
            client.execute(
                "send-key",
                json!({"keys":[{"type":"qcode","data":"p"}],"hold-time":100}),
            )?;
            let panic_deadline = Instant::now() + Duration::from_secs(5);
            while !fs::read_to_string(&prepared.serial)
                .unwrap_or_default()
                .contains("DOOR_DEMO PANIC_SCREEN_READY")
            {
                child.poll()?;
                if Instant::now() > panic_deadline {
                    return Err("interactive panic screen missing".into());
                }
                thread::sleep(Duration::from_millis(20));
            }
            let panic_screen = prepared.private.join("panic.png");
            client.execute(
                "screendump",
                json!({"filename":file_path(&panic_screen)?,"format":"png"}),
            )?;
            if file_hash(&panic_screen)? == original_hash
                || file_hash(&panic_screen)? == diagnostics_hash
            {
                return Err("panic screen unchanged".into());
            }
            interactions.push(json!({"key":"p","marker":"DOOR_DEMO PANIC_SCREEN_READY"}));
            client.execute("quit", json!({}))?;
            if !child.wait()?.success() { return Err("demo QEMU exit failed".into()); }
        }
        crate::image::validate_staged(
            &root,
            &relative(&root, &prepared.private.join("image.json"))?,
        )?;
        emulator::acquired_inputs(&prepared.profile)?;
        let additional_screens = if exercise {
            vec!["diagnostics.png", "panic.png"]
                .into_iter()
                .map(|name| {
                    let path = prepared.private.join(name);
                    Ok(json!({"path":relative(&root,&path)?,"sha256":file_hash(&path)?}))
                })
                .collect::<Result<Vec<_>, String>>()?
        } else {
            Vec::new()
        };
        let record = json!({"schema_version":1,"record_kind":"demo_session","pid":child.id(),"qmp_port":port,"uuid":uuid,"run_token":token,"visible":visible,"argv":args,"profile":prepared.profile,"image":prepared.image_record,"boot_medium":{"path":relative(&root,&prepared.private.join("esp"))?,"kind":"read-only-vvfat-via-virtio"},"private_directory":relative(&root,&prepared.private)?,"private_vars":relative(&root,&prepared.vars)?,"screenshot":{"path":relative(&root,&screenshot)?,"sha256":original_hash},"additional_screens":additional_screens,"serial":{"path":relative(&root,&prepared.serial)?,"sha256_at_ready":file_hash(&prepared.serial)?},"interactions":interactions,"state":if exercise{"completed"}else{"running"},"scope":"real kernel diagnostic console; no desktop or applications"});
        if !exercise { child.handoff()?; }
        Ok(record)
    })?;
    record["process_command"] = run.commands().last().ok_or("demo process missing")?["id"].clone();
    record["private_vars_sha256"] = json!(file_hash(&prepared.vars)?);
    write_json(&directory.join("demo-session.json"), &record)?;
    if !exercise {
        write_json(&root.join("out/demo-session.json"), &record)?;
    }
    Ok(record)
}
pub fn arguments(
    mut base: Vec<String>,
    visible: bool,
    port: u16,
    uuid: &str,
    phase: &str,
) -> Result<Vec<String>, String> {
    let title = match phase {
        "P02" => "Door OS - P02 boot diagnostics",
        "P03" => "Door OS - P03 memory and CPU",
        "P04" => "Door OS - P04 scheduler and SMP",
        "P05" => "Door OS - P05 protected native execution",
        "P06" => "Door OS - P06 storage foundations",
        "P07" => "Door OS - P07 durable namespace and recovery",
        "P08" => "Door OS - P08 cumulative diagnostics",
        "P09" => "Door OS - P09 cumulative diagnostics",
        "P10" => "Door OS - P10 cumulative diagnostics",
        "P11" => "Door OS - P11 cumulative diagnostics",
        "P12" => "Door OS - P12 cumulative diagnostics",
        "P13" => "Door OS - P13 cumulative diagnostics",
        "P14" => "Door OS - P14 cumulative diagnostics",
        "P15" => "Door OS - P15 cumulative diagnostics",
        "P16" => "Door OS - P16 cumulative diagnostics",
        "P17" => "Door OS - P17 cumulative diagnostics",
        _ => return Err("demo phase is not implemented".into()),
    };
    if port == 0 || uuid.len() != 36 || !uuid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err("invalid demo control identity".into());
    }
    base.retain(|arg| arg != "-no-reboot");
    let position = base
        .iter()
        .position(|arg| arg == "-display")
        .ok_or("display argument")?;
    *base.get_mut(position + 1).ok_or("display value")? =
        if visible { "sdl" } else { "none" }.into();
    base.extend([
        "-name".into(),
        title.into(),
        "-uuid".into(),
        uuid.into(),
        "-qmp".into(),
        format!("tcp:127.0.0.1:{port},server=on,wait=off"),
        "-fw_cfg".into(),
        "name=opt/door/mode,string=demo".into(),
    ]);
    Ok(base)
}
pub fn stop(run: &Run) -> Result<(), String> {
    let mut record = crate::image::read_json(&run.root.join("out/demo-session.json"))?;
    if record["state"] != "running" {
        return Err("no running diagnostic session".into());
    }
    let port = u16::try_from(record["qmp_port"].as_u64().ok_or("session port")?)
        .map_err(|e| e.to_string())?;
    let mut client = qmp::Client::connect(port)?;
    if client.execute("query-uuid", json!({}))?["UUID"] != record["uuid"] {
        return Err("session UUID differs; cannot stop another process".into());
    }
    client.execute("quit", json!({}))?;
    record["state"] = json!("stopped");
    write_json(&run.root.join("out/demo-session.json"), &record)
}
