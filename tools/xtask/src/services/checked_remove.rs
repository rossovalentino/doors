//! Private native checked deletion, real pending snapshot and separate reboot.
use super::{PROFILE, audit, demo, scenarios};
use crate::{
    emulator,
    evidence::{Run, file_hash, relative, write_json},
    paths,
    persistence::{catalog, journal, tree},
    qmp,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, net::TcpListener, path::Path, thread, time::Duration};
const PAYLOAD: &[u8] = b"Removal binary\0\xff\n";
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}

fn snapshot(baseline: &Path, path: &Path, state: &str) -> Result<Value, String> {
    let old = fs::read(baseline).map_err(|e| e.to_string())?;
    let new = fs::read(path).map_err(|e| e.to_string())?;
    require(
        old.len() == crate::storage::DISK_BYTES && new.len() == old.len(),
        "remove disk length",
    )?;
    crate::persistence::gpt(&new)?;
    let a = journal::inspect(&old)?;
    let b = journal::inspect(&new)?;
    require(!b.selected().committed, "remove checkpoint not clean")?;
    let ah = a.replayed_home(&old)?;
    let bh = b.replayed_home(&new)?;
    let at = tree::inspect(&ah)?;
    let bt = tree::inspect(&bh)?;
    let ac = catalog::decode(&at)?.ok_or("remove baseline catalog")?;
    let bc = if state == "pending" {
        catalog::decode_pending(&bt)?
    } else {
        catalog::decode(&bt)?
    }
    .ok_or("remove catalog")?;
    let ap = ac.paths()?;
    let bp = bc.paths()?;
    require(
        bc.next_object == ac.next_object + 3 && bc.next_key == ac.next_key,
        "remove exact allocated object/key counters",
    )?;
    let revision = match state {
        "prepared" => 5,
        "pending" => 11,
        "final" => 12,
        "reclaimed" => 11,
        _ => return Err("remove audit state".into()),
    };
    // Reboot reclaim removes a persisted pending object without allocating a
    // new live catalog revision. It retains pending's +11 revision.
    require(
        bc.revision == ac.revision + revision,
        "remove exact catalog revision",
    )?;
    let mut allowed = vec![false; ac.bytes.len()];
    for i in (16..24).chain(32..44) {
        allowed[i] = true;
    }
    // A binding change stamps the folder modified time (catalog v4/v5).
    if let Some(folder) = ap.get("/users/1001")
        && let Some((start, _)) = catalog::record_offset(ac.bytes, folder.id)
    {
        allowed[start + 216..start + 224].fill(true);
    }
    let mut extras = Vec::new();
    for (name, node) in &bp {
        if ap.contains_key(name) {
            continue;
        }
        let expected = match name.as_str() {
            "/users/1001/remove.bin" => (1, PAYLOAD, 0),
            "/users/1001/remove-folder" if state == "prepared" => (2, &[][..], 1),
            "/users/1001/remove-folder/child.bin" if state == "prepared" => {
                (1, &b"Pending child\n"[..], 2)
            }
            _ => return Err("remove unexpected extra namespace".into()),
        };
        require(
            matches!(state, "prepared" | "pending")
                && node.id == ac.next_object + expected.2
                && node.owner == 1001
                && node.kind == expected.0
                && node.pending == (state == "pending"),
            "remove exact extra identity/state",
        )?;
        let index = bc.bytes[128..7296]
            .chunks_exact(224)
            .position(|r| u64::from_le_bytes(r[..8].try_into().unwrap()) == node.id)
            .ok_or("remove extra record")?;
        let range = 128 + index * 224..128 + (index + 1) * 224;
        let accessed = catalog::accessed_offset(index);
        allowed[accessed..accessed + 8].fill(true);
        let version = u64::from_le_bytes(
            bc.bytes[range.start + 24..range.start + 32]
                .try_into()
                .unwrap(),
        );
        require(
            version
                == if state == "pending" {
                    4
                } else if node.kind == 1 {
                    2
                } else {
                    1
                },
            "remove exact observed node version",
        )?;
        require(
            ac.bytes[range.clone()].iter().all(|x| *x == 0),
            "remove extra reused existing node",
        )?;
        allowed[range].fill(true);
        if node.kind == 1 {
            require(
                bt.files[&format!("obj{:016x}", node.id)].bytes == expected.1,
                "remove exact temporary payload",
            )?;
        }
        extras.push(json!({"path":name,"object":node.id,"version":version,"pending":node.pending,"inode":node.inode,"generation":node.generation,"bytes":expected.1.len()}));
    }
    require(
        extras.len()
            == match state {
                "prepared" => 3,
                "pending" => 1,
                _ => 0,
            },
        "remove exact extra node count",
    )?;
    require(
        bp.len() == ap.len() + extras.len(),
        "remove lost baseline namespace",
    )?;
    require(
        ac.bytes.len() == bc.bytes.len()
            && ac
                .bytes
                .iter()
                .zip(bc.bytes)
                .enumerate()
                .all(|(i, (a, b))| a == b || allowed[i]),
        "remove changed unrelated catalog bytes/ACL/settings",
    )?;
    for (name, file) in &at.files {
        let next = bt.files.get(name).ok_or("remove lost unrelated backing")?;
        require(
            file.inode == next.inode
                && file.generation == next.generation
                && file.blocks == next.blocks
                && (name == "door.catalog" || file.bytes == next.bytes),
            "remove changed unrelated backing identity/payload/allocation",
        )?;
    }
    let files = if state == "prepared" {
        2
    } else if state == "pending" {
        1
    } else {
        0
    };
    require(
        bt.files.len() == at.files.len() + files
            && bt.free_blocks + files as u64 == at.free_blocks
            && bt.free_inodes + files as u64 == at.free_inodes,
        "remove exact allocation graph/counters",
    )?;
    require(
        old.iter().zip(&new).enumerate().all(|(i, (a, b))| {
            a == b
                || (133120 * 512..198656 * 512).contains(&i)
                || (202752 * 512..206848 * 512).contains(&i)
        }),
        "remove changed bytes outside home/journal",
    )?;
    Ok(
        json!({"path":path,"sha256":file_hash(path)?,"state":state,"catalog_revision":bc.revision,"next_object":bc.next_object,"journal":b.summary(),"extras":extras,"free_blocks":bt.free_blocks,"free_inodes":bt.free_inodes,"exact_unrelated_catalog_and_payload":true,"independent_allocation_graph":true}),
    )
}
fn markers(text: &str, first: bool, checkpoints: &[Value]) -> Result<(), String> {
    let commands: &[&str] = if first {
        &["removeprep", "removefiles", "threads", "exit"]
    } else {
        &["removeverify", "threads", "exit"]
    };
    let processes = scenarios::validate_commands(text, scenarios::Mode::Demo, commands)
        .ok_or("remove kernel process/command/retirement attribution")?;
    let app = processes
        .iter()
        .find(|p| p.manifest == 5 && p.uid == 1001 && p.exit == 71)
        .ok_or("remove SDK authority")?;
    for (marker, count) in [
        (
            "DOOR_P12 SDK REMOVE PREPARED uid=1001 file=1 folder=1 child=1",
            usize::from(first),
        ),
        (
            "DOOR_P12 SDK REMOVE PASS uid=1001 malformed=28 stale=1 foreign=1 protected=1 sharing=1 rename_utf16=1 pending=1 empty=1 nonempty=1",
            usize::from(first),
        ),
        (
            "DOOR_P12 SDK REMOVE VERIFIED uid=1001 absent=3 neighbor=4096",
            1,
        ),
    ] {
        let lines: Vec<_> = text
            .lines()
            .filter(|l| l.contains(marker.split(" uid=").next().unwrap()))
            .collect();
        require(
            lines.len() == count
                && lines
                    .iter()
                    .all(|l| *l == format!("DOOR_USER pid={} {marker}", app.pid)),
            "remove exact attributable marker",
        )?;
    }
    let pending: Vec<_> = text
        .lines()
        .filter(|l| l.contains("DOOR_P12 SDK REMOVE PENDING"))
        .collect();
    require(
        pending.len() == usize::from(first),
        "remove pending marker count",
    )?;
    if first {
        let raw = checkpoints
            .iter()
            .find(|x| x["state"] == "pending")
            .ok_or("remove raw pending checkpoint")?;
        let item = &raw["extras"][0];
        let object = item["object"].as_u64().ok_or("remove pending raw object")?;
        let version = item["version"]
            .as_u64()
            .ok_or("remove pending raw version")?;
        require(
            pending[0]
                == format!(
                    "DOOR_USER pid={} DOOR_P12 SDK REMOVE PENDING uid=1001 object={object} version={version} bytes=17",
                    app.pid
                ),
            "remove authenticated pending identity differs from raw disk",
        )?;
        let prepared = checkpoints
            .iter()
            .find(|x| x["state"] == "prepared")
            .and_then(|x| x["extras"].as_array())
            .and_then(|x| x.iter().find(|n| n["object"] == item["object"]))
            .ok_or("remove prepared identity")?;
        require(
            prepared["inode"] == item["inode"]
                && prepared["generation"] == item["generation"]
                && prepared["bytes"] == item["bytes"],
            "remove changed live pending backing identity",
        )?;
    }
    let device: Vec<_> = text
        .lines()
        .filter(|l| l.starts_with("DOOR_P08 DEVICE "))
        .collect();
    require(
        device.len() == 1 && device[0].ends_with(" reclaimed=8"),
        "remove DMA retirement",
    )
}
fn retain(
    root: &Path,
    directory: &Path,
    client: &mut qmp::Client,
    disk: &Path,
    baseline: &Path,
    state: &str,
) -> Result<Value, String> {
    client.execute("stop", json!({}))?;
    let path = directory.join(format!("remove-{state}.raw"));
    let copied = fs::copy(disk, &path).map_err(|e| e.to_string());
    let resumed = client.execute("cont", json!({}));
    copied?;
    resumed?;
    let mut record = snapshot(baseline, &path, state)?;
    record["path"] = json!(relative(root, &path)?);
    Ok(record)
}
fn boot(
    run: &mut Run,
    image: &str,
    disk: &Path,
    baseline: &Path,
    kind: &str,
) -> Result<Value, String> {
    let first = kind == "live";
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
    let commands: &[&str] = if first {
        &["removeprep", "removefiles", "threads", "exit"]
    } else {
        &["removeverify", "threads", "exit"]
    };
    let root = run.root.clone();
    let directory = run.directory.clone();
    let result=run.controlled_expected(prepared.qemu.to_str().ok_or("QEMU path")?,&args,&[(&prepared.serial,1024*1024)],33,|child| {
        demo::wait_marker(child,&prepared.serial,"DOOR_P08 CONSOLE READY",35)?;
        let mut client=qmp::Client::connect(port)?;
        require(client.execute("query-uuid",json!({}))?["UUID"]==uuid,"remove QMP identity")?;
        let mut checkpoints: Vec<Value>=Vec::new();
        for command in commands {
            for key in command.chars().map(|c|c.to_string()).chain(std::iter::once("ret".into())) {
                client.execute("send-key",json!({"keys":[{"type":"qcode","data":key}],"hold-time":20}))?; thread::sleep(Duration::from_millis(35));
            }
            if *command=="removefiles" {
                let item=checkpoints.iter().find(|x|x["state"]=="prepared").and_then(|x|x["extras"].as_array()).and_then(|x|x.iter().find(|n|n["path"]=="/users/1001/remove.bin")).ok_or("remove prepared target")?;
                let object=item["object"].as_u64().ok_or("remove prepared target identity")?;
                demo::wait_marker(child,&prepared.serial,&format!("DOOR_P12 SDK REMOVE PENDING uid=1001 object={object} version=4 bytes=17"),30)?;
                checkpoints.push(retain(&root,&directory,&mut client,disk,baseline,"pending")?);
                client.execute("send-key",json!({"keys":[{"type":"qcode","data":"p"}],"hold-time":20}))?;
            }
            demo::wait_marker(child,&prepared.serial,&format!("DOOR_P08 CONSOLE COMMAND {command}"),30)?;
            if *command=="removeprep" { checkpoints.push(retain(&root,&directory,&mut client,disk,baseline,"prepared")?); }
            if *command=="removefiles" { checkpoints.push(retain(&root,&directory,&mut client,disk,baseline,"final")?); }
        }
        require(child.wait()?.code()==Some(33),"remove actual debug exit")?;
        let text=fs::read_to_string(&prepared.serial).map_err(|e|e.to_string())?;markers(&text,first,&checkpoints)?;
        let final_path=directory.join("after.raw");fs::copy(disk,&final_path).map_err(|e|e.to_string())?;
        let mut final_audit=snapshot(baseline,&final_path,if kind=="pending-reboot" {"reclaimed"}else {"final"})?;final_audit["path"]=json!(relative(&root,&final_path)?);
        Ok(json!({"commands":commands,"checkpoints":checkpoints,"final":final_audit,"serial":{"path":relative(&root,&prepared.serial)?,"sha256":file_hash(&prepared.serial)?},"argv":args,"profile":prepared.profile,"image":prepared.image_record,"vars":{"path":relative(&root,&prepared.vars)?,"sha256":file_hash(&prepared.vars)?},"qmp_port":port,"uuid":uuid,"exit_code":33,"visible":false,"processes":5,"threads":7,"leaked":0,"device_pages":8}))
    })?;
    crate::image::validate_staged(
        &root,
        &relative(&root, &prepared.private.join("image.json"))?,
    )?;
    emulator::acquired_inputs(&prepared.profile)?;
    Ok(result)
}
pub fn run(run: &mut Run, image: &str, baseline: &str) -> Result<(), String> {
    let baseline_path = paths::existing(&run.root, baseline)?;
    audit::inspect(&baseline_path)?;
    let baseline_hash = file_hash(&baseline_path)?;
    let directory =
        paths::fresh_directory(&run.root, &format!("out/storage/{}-checked-remove", run.id))?;
    let disk = directory.join("data.raw");
    fs::copy(&baseline_path, &disk).map_err(|e| e.to_string())?;
    let mut boots = Vec::new();
    let mut pending = None;
    for kind in ["live", "read-only", "pending-reboot"] {
        let active = if kind == "pending-reboot" {
            directory.join("pending-reboot.raw")
        } else {
            disk.clone()
        };
        if kind == "pending-reboot" {
            fs::copy(pending.as_ref().ok_or("remove pending snapshot")?, &active)
                .map_err(|e| e.to_string())?;
        }
        let before = file_hash(&active)?;
        let mut child = Run::new(run.root.clone())?;
        let result = boot(&mut child, image, &active, &baseline_path, kind);
        child.finish(result.as_ref().err().map(String::as_str))?;
        let mut record = result?;
        if kind == "live" {
            let p = record["checkpoints"]
                .as_array()
                .ok_or("remove checkpoints")?
                .iter()
                .find(|x| x["state"] == "pending")
                .ok_or("remove pending checkpoint")?;
            pending = Some(paths::existing(
                &run.root,
                p["path"].as_str().ok_or("remove pending path")?,
            )?);
        }
        if kind == "read-only" {
            require(
                file_hash(&active)? == before,
                "remove read-only boot changed disk",
            )?;
        }
        record["kind"] = json!(kind);
        record["report"] = json!({"path":relative(&run.root,&child.directory.join("report.json"))?,"sha256":file_hash(&child.directory.join("report.json"))?});
        boots.push(record);
    }
    require(
        file_hash(&baseline_path)? == baseline_hash,
        "remove mutated baseline",
    )?;
    let suite = json!({"schema_version":1,"record_kind":"sdk_checked_remove_suite","outcome":"Passed","verification_state":"guest_tested","baseline":{"path":baseline,"sha256":baseline_hash},"image":image,"boots":boots,"owner_disk_used":false,"scope":"One authenticated UID, exact wire validation/current foreign ancestor denial, same-UID sharing, live pending stream and actual pending snapshot reboot reclamation; host-only fault injection remains separate"});
    write_json(&run.directory.join("sdk-checked-remove.json"), &suite)?;
    run.record_guest(&suite)?;
    run.check("p12-sdk-checked-remove", suite);
    Ok(())
}
