//! Private native reconnect exercise. New raw evidence lives under reports/runs;
//! existing producer layouts and historical receipts remain unchanged.
use super::{PROFILE, audit, demo, scenarios};
use crate::{
    emulator,
    evidence::{Run, file_hash, relative, write_json},
    paths, qmp,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, net::TcpListener, path::Path, thread, time::Duration};

const MARKER: &str = "DOOR_P12 STORAGE_CONNECT PASS uid=1001 cycles=64 stale_file=1 stale_key=1 stale_stage=1 unchanged=1";
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn markers(text: &str, commands: &[&str], exercised: bool) -> Result<(), String> {
    let processes = scenarios::validate_commands(text, scenarios::Mode::Demo, commands)
        .ok_or("reconnect SDK authenticated lifecycle/commands")?;
    let matches: Vec<_> = text
        .lines()
        .filter(|line| line.contains("DOOR_P12 STORAGE_CONNECT"))
        .collect();
    let app = processes
        .iter()
        .find(|p| p.manifest == 5 && p.uid == 1001 && p.exit == 71)
        .ok_or("reconnect SDK retired identity")?;
    require(
        if exercised {
            matches == [format!("DOOR_USER pid={} {MARKER}", app.pid)]
                && marker_lifetime(text, app.pid)
        } else {
            matches.is_empty()
        },
        "reconnect SDK exact owner/count/fields",
    )
}
fn marker_lifetime(text: &str, pid: u64) -> bool {
    let lines: Vec<_> = text.lines().collect();
    let unique = |wanted: &str| {
        let mut positions = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| **line == wanted);
        let first = positions.next()?.0;
        positions.next().is_none().then_some(first)
    };
    let ready = unique(&format!("DOOR_USER pid={pid} DOOR_P08 CONSOLE READY"));
    let passed = unique(&format!("DOOR_USER pid={pid} {MARKER}"));
    let done = unique(&format!(
        "DOOR_USER pid={pid} DOOR_P08 CONSOLE COMMAND reconnect"
    ));
    let retired = lines
        .iter()
        .position(|line| scenarios::process(line).is_some_and(|p| p.pid == pid));
    let cleanup = lines
        .iter()
        .position(|line| line.starts_with("DOOR_P08 CLEANUP PASS "));
    matches!((ready,passed,done,retired,cleanup), (Some(a),Some(b),Some(c),Some(d),Some(e)) if a < b && b < c && c < d && d < e)
}
fn boot(
    run: &mut Run,
    image: &str,
    disk: &Path,
    baseline_hash: &str,
    exercised: bool,
) -> Result<Value, String> {
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
    let args = demo::arguments(prepared.args, disk, &prepared.profile, false, port, &uuid)?;
    let commands: &[&str] = if exercised {
        &["reconnect", "file", "threads", "exit"]
    } else {
        &["file", "threads", "exit"]
    };
    let root = run.root.clone();
    let directory = run.directory.clone();
    let token = run.id.clone();
    let mut result = run.controlled_expected(prepared.qemu.to_str().ok_or("QEMU path")?, &args, &[(&prepared.serial, 1024*1024)], 33, |child| {
        demo::wait_marker(child, &prepared.serial, "DOOR_P08 CONSOLE READY", 35)?;
        let mut client = qmp::Client::connect(port)?;
        require(client.execute("query-uuid", json!({}))?["UUID"] == uuid, "reconnect SDK QMP identity")?;
        for command in commands {
            for key in command.chars().map(|c| c.to_string()).chain(std::iter::once("ret".into())) {
                client.execute("send-key", json!({"keys":[{"type":"qcode","data":key}],"hold-time":20}))?;
                thread::sleep(Duration::from_millis(35));
            }
            demo::wait_marker(child, &prepared.serial, &format!("DOOR_P08 CONSOLE COMMAND {command}"), 30)?;
        }
        require(child.wait()?.code() == Some(33), "reconnect SDK actual debug exit")?;
        let text = fs::read_to_string(&prepared.serial).map_err(|e| e.to_string())?;
        markers(&text, commands, exercised)?;
        require(emulator::classify(Some(33), false, false, false, false, &text, &token) == emulator::Outcome::Passed, "reconnect SDK boot token/outcome")?;
        let after = directory.join("storage-connect-after.raw");
        fs::copy(disk, &after).map_err(|e| e.to_string())?;
        audit::inspect(&after)?;
        require(file_hash(&after)? == baseline_hash, "reconnect SDK changed persistent bytes")?;
        Ok(json!({"run_token":token,"boot_medium":{"path":relative(&root,&prepared.private.join("esp"))?,"kind":"read-only-vvfat-via-virtio"},"private_vars":relative(&root,&prepared.vars)?,"private_vars_sha256":file_hash(&prepared.vars)?,"commands":commands,"exercised":exercised,"serial":{"path":relative(&root,&prepared.serial)?,"sha256":file_hash(&prepared.serial)?},"argv":args,"profile":prepared.profile,"image":prepared.image_record,"qmp_port":port,"uuid":uuid,"exit_code":33,"visible":false,"final":{"path":relative(&root,&after)?,"sha256":file_hash(&after)?},"processes":5,"threads":7,"leaked":0,"device_pages":8}))
    })?;
    crate::image::validate_staged(
        &root,
        &relative(&root, &prepared.private.join("image.json"))?,
    )?;
    emulator::acquired_inputs(&prepared.profile)?;
    result["process_command"] = run
        .commands()
        .last()
        .ok_or("reconnect SDK command receipt")?["id"]
        .clone();
    Ok(result)
}
pub fn run(run: &mut Run, image: &str, baseline: &str) -> Result<(), String> {
    let saved = paths::existing(&run.root, baseline)?;
    audit::inspect(&saved)?;
    let baseline_hash = file_hash(&saved)?;
    let disk = run.directory.join("storage-connect-private.raw");
    require(!disk.exists(), "reconnect SDK private disk already exists")?;
    fs::copy(&saved, &disk).map_err(|e| e.to_string())?;
    let mut boots = Vec::new();
    for exercised in [true, false] {
        let mut child = Run::new(run.root.clone())?;
        let result = boot(&mut child, image, &disk, &baseline_hash, exercised);
        child.finish(result.as_ref().err().map(String::as_str))?;
        let mut record = result?;
        let sealed = crate::image::read_json(&child.directory.join("report.json"))?;
        crate::boot_gate::logs(&run.root, &sealed, Some((&record["process_command"], 33)))?;
        let (_, staged) = crate::image::validate_staged(&run.root, image)?;
        let base = crate::boot_gate::private_launch(&run.root, &record, &sealed, &staged, PROFILE)?;
        let port = u16::try_from(
            record["qmp_port"]
                .as_u64()
                .ok_or("reconnect SDK QMP port")?,
        )
        .map_err(|e| e.to_string())?;
        let expected = demo::arguments(
            base,
            &disk,
            &record["profile"],
            false,
            port,
            record["uuid"].as_str().ok_or("reconnect SDK UUID")?,
        )?;
        require(
            record["argv"] == json!(expected),
            "reconnect SDK declared argv changed",
        )?;
        let process = crate::boot_gate::process(&sealed, &record["profile"], &expected)?;
        require(
            process["id"] == record["process_command"],
            "reconnect SDK actual process binding",
        )?;
        record["report"] = json!({"path":relative(&run.root,&child.directory.join("report.json"))?,"sha256":file_hash(&child.directory.join("report.json"))?});
        boots.push(record);
    }
    require(
        file_hash(&saved)? == baseline_hash,
        "reconnect SDK mutated source baseline",
    )?;
    let suite = json!({"schema_version":1,"record_kind":"sdk_storage_connect_suite","outcome":"Passed","verification_state":"guest_tested","baseline":{"path":baseline,"sha256":baseline_hash},"image":image,"boots":boots,"owner_disk_used":false,"evidence_layout":"raw-v1-reports-runs; existing QEMU private layout unchanged","scope":"64 explicit client retirements and polled authenticated resolves, stale file/key/staging rejection, broker capacity reclamation and unchanged persistent bytes across two actual boots; no native malformed/timeout injection or security-session-loss survival claim"});
    write_json(&run.directory.join("sdk-storage-connect.json"), &suite)?;
    run.record_guest(&suite)?;
    run.check("p12-sdk-storage-connect", suite);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn success_must_happen_inside_the_live_ready_command_before_retirement() {
        let rows = vec![
            "DOOR_USER pid=5 DOOR_P08 CONSOLE READY".to_owned(),
            format!("DOOR_USER pid=5 {MARKER}"),
            "DOOR_USER pid=5 DOOR_P08 CONSOLE COMMAND reconnect".to_owned(),
            "DOOR_P08 PROCESS manifest=5 pid=5 instance=9 uid=1001 session=3 epoch=1 exit=71"
                .to_owned(),
            "DOOR_P08 CLEANUP PASS processes=5 threads=7 leaked=0".to_owned(),
        ];
        assert!(marker_lifetime(&rows.join("\n"), 5));
        for at in [0, 2, 3, 4] {
            let mut invalid = rows.clone();
            invalid.swap(1, at);
            assert!(!marker_lifetime(&invalid.join("\n"), 5));
        }
        assert!(!marker_lifetime(
            &format!("{}\n{}", rows.join("\n"), rows[1]),
            5
        ));
        assert!(!marker_lifetime(&rows.join("\n"), 6));
    }
    #[test]
    fn marker_without_authenticated_retirement_is_rejected() {
        for text in [
            MARKER.to_owned(),
            format!("DOOR_USER pid=1 {MARKER}"),
            format!("DOOR_USER pid=1 {MARKER}\nDOOR_USER pid=1 {MARKER}"),
        ] {
            assert!(markers(&text, &["reconnect", "file", "threads", "exit"], true).is_err());
        }
    }
}
