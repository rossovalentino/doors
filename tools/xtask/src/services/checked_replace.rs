//! Separate SDK regression; historical service/rename policies remain strict.
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
use std::{collections::BTreeSet, fs, net::TcpListener, path::Path, thread, time::Duration};
const SAVED: &[u8] = b"Checked atomic document\0\xff\n";
const REPLACE_MARKER: &str = "DOOR_P12 SDK REPLACE PASS uid=1001 versions=4 private=1 stale=1 grant=1 capacity=1 close=1 max=1 empty=1";
const WIRE_MARKER: &str = "DOOR_P12 SDK REPLACE WIRE PASS uid=1001 foreign_file=1 foreign_stage=1 chunk=1 commit=1 abort=1 incomplete=1 offset=1";
const RESTORE_MARKER: &str = "DOOR_P12 SDK RESTORE PASS uid=1001 bytes=4096 versions=1 identity=1";
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn word(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
fn original() -> Vec<u8> {
    (0..4096).map(|i| ((i * 29 + 17) % 251) as u8).collect()
}
fn snapshot(baseline: &Path, path: &Path, expected: &[u8], versions: u64) -> Result<Value, String> {
    let old = fs::read(baseline).map_err(|e| e.to_string())?;
    let new = fs::read(path).map_err(|e| e.to_string())?;
    require(
        old.len() == crate::storage::DISK_BYTES && new.len() == old.len(),
        "SDK replacement disk length",
    )?;
    crate::persistence::gpt(&new)?;
    let old_journal = journal::inspect(&old)?;
    let new_journal = journal::inspect(&new)?;
    require(
        !new_journal.selected().committed
            && new_journal.selected().generation == old_journal.selected().generation + versions,
        "SDK replacement exact clean journal generation",
    )?;
    let old_home = old_journal.replayed_home(&old)?;
    let new_home = new_journal.replayed_home(&new)?;
    let old_tree = tree::inspect(&old_home)?;
    let new_tree = tree::inspect(&new_home)?;
    let old_cat = catalog::decode(&old_tree)?.ok_or("SDK replacement baseline catalog")?;
    let new_cat = catalog::decode(&new_tree)?.ok_or("SDK replacement final catalog")?;
    let old_paths = old_cat.paths()?;
    let source = old_paths
        .get("/users/1001/workload.bin")
        .ok_or("SDK replacement source")?;
    let index = old_cat.bytes[128..7296]
        .chunks_exact(224)
        .position(|r| word(r, 0) == source.id)
        .ok_or("SDK replacement source record")?;
    let version = 128 + index * 224 + 24;
    // Catalog v4/v5: the replacement stamps the file's modified time and
    // access entry, nothing else.
    let modified = 128 + index * 224 + 216;
    let accessed = catalog::accessed_offset(index);
    require(
        new_cat.revision == old_cat.revision + versions
            && word(new_cat.bytes, version) == word(old_cat.bytes, version) + versions,
        "SDK replacement exact catalog/source revisions",
    )?;
    require(
        old_cat.bytes.len() == new_cat.bytes.len()
            && old_cat
                .bytes
                .iter()
                .zip(new_cat.bytes)
                .enumerate()
                .all(|(at, (a, b))| {
                    a == b
                        || (32..44).contains(&at)
                        || (version..version + 8).contains(&at)
                        || (modified..modified + 8).contains(&at)
                        || (accessed..accessed + 8).contains(&at)
                }),
        "SDK replacement changed unrelated namespace, ACL, setting or registry",
    )?;
    let name = format!("obj{:016x}", source.id);
    let before = &old_tree.files[&name];
    let after = &new_tree.files[&name];
    require(
        before.inode == after.inode
            && before.generation == after.generation
            && before.blocks == after.blocks
            && before.blocks.len() == 1
            && after.bytes == expected,
        "SDK replacement identity/allocation/complete payload",
    )?;
    require(
        old_tree.files.len() == new_tree.files.len(),
        "SDK replacement file count",
    )?;
    for (name, file) in &old_tree.files {
        let next = new_tree
            .files
            .get(name)
            .ok_or("SDK replacement removed unrelated backing")?;
        require(
            file.inode == next.inode
                && file.generation == next.generation
                && file.blocks == next.blocks,
            "SDK replacement changed backing identity",
        )?;
        if name != "door.catalog" && *name != format!("obj{:016x}", source.id) {
            require(
                file.bytes == next.bytes,
                "SDK replacement changed unrelated payload",
            )?;
        }
    }
    let backing = &old_tree.files["door.catalog"];
    let physical = |at: usize| backing.blocks[at / 4096] * 4096 + at % 4096;
    let mut allowed: BTreeSet<usize> = (32..44).chain(version..version + 8).map(physical).collect();
    let data = before.blocks[0] * 4096;
    allowed.extend(data..data + 4096);
    let inode = 4 * 4096 + (before.inode as usize - 1) * 128;
    allowed.extend(inode + 4..inode + 8);
    require(
        new_home[data + expected.len()..data + 4096]
            .iter()
            .all(|b| *b == 0),
        "SDK replacement retained nonzero bytes beyond EOF",
    )?;
    require(
        old_home
            .iter()
            .zip(&new_home)
            .enumerate()
            .all(|(at, (a, b))| a == b || allowed.contains(&at)),
        "SDK replacement changed unrelated home bytes",
    )?;
    require(
        old.iter().zip(&new).enumerate().all(|(at, (a, b))| {
            a == b
                || (133120 * 512..198656 * 512).contains(&at)
                || (202752 * 512..206848 * 512).contains(&at)
        }),
        "SDK replacement changed GPT/FAT/gap/outside home and journal",
    )?;
    Ok(
        json!({"path":path,"sha256":file_hash(path)?,"bytes":expected.len(),"payload_sha256":format!("{:x}",Sha256::digest(expected)),"object":source.id,"inode":after.inode,"generation":after.generation,"version":word(new_cat.bytes,version),"catalog_revision":new_cat.revision,"journal":new_journal.summary(),"unrelated_bytes_unchanged":true,"complete_expected_bytes":true}),
    )
}
fn markers(serial: &str, commands: &[&str]) -> Result<(), String> {
    let processes = scenarios::validate_commands(serial, scenarios::Mode::Demo, commands)
        .ok_or("SDK replacement kernel attribution/retirement")?;
    let owner = processes
        .iter()
        .find(|p| p.manifest == 5 && p.uid == 1001 && p.exit == 71)
        .ok_or("SDK replacement authenticated app")?;
    for marker in [REPLACE_MARKER, WIRE_MARKER, RESTORE_MARKER] {
        let matches: Vec<_> = serial
            .lines()
            .filter(|l| l.contains(&format!("{} PASS ", marker.split(" PASS").next().unwrap())))
            .collect();
        let expected = if commands.contains(&"replacedoc") {
            1
        } else {
            0
        };
        require(
            matches.len() == expected
                && matches
                    .iter()
                    .all(|l| *l == format!("DOOR_USER pid={} {marker}", owner.pid)),
            "SDK replacement exact attributable marker",
        )?;
    }
    let device: Vec<_> = serial
        .lines()
        .filter(|l| l.starts_with("DOOR_P08 DEVICE "))
        .collect();
    require(
        device.len() == 1 && device[0].ends_with(" reclaimed=8"),
        "SDK replacement complete DMA retirement",
    )
}
fn boot(
    run: &mut Run,
    image: &str,
    disk: &Path,
    baseline: &Path,
    first: bool,
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
    let commands: &[&str] = if first {
        &["replacedoc", "restoredoc", "threads", "exit"]
    } else {
        &["file", "threads", "exit"]
    };
    let root = run.root.clone();
    let directory = run.directory.clone();
    let result=run.controlled_expected(prepared.qemu.to_str().ok_or("QEMU path")?,&args,&[(&prepared.serial,1024*1024)],33,|child| {
        demo::wait_marker(child,&prepared.serial,"DOOR_P08 CONSOLE READY",35)?;
        let mut client=qmp::Client::connect(port)?;require(client.execute("query-uuid",json!({}))?["UUID"]==uuid,"SDK replacement QMP identity")?;
        let mut checkpoints=Vec::new();
        for command in commands {
            for key in command.chars().map(|c|c.to_string()).chain(std::iter::once("ret".into())) {
                client.execute("send-key",json!({"keys":[{"type":"qcode","data":key}],"hold-time":20}))?;thread::sleep(Duration::from_millis(35));
            }
            demo::wait_marker(child,&prepared.serial,&format!("DOOR_P08 CONSOLE COMMAND {command}"),30)?;
            if !matches!(*command,"exit"|"threads") {
                client.execute("stop",json!({}))?;
                let path=directory.join(format!("{command}.raw"));let copied=fs::copy(disk,&path).map_err(|e|e.to_string());let resumed=client.execute("cont",json!({}));copied?;resumed?;
                let expected=if *command=="replacedoc" {SAVED.to_vec()}else{original()};let versions=if *command=="replacedoc" {4}else{5};
                let mut audit=snapshot(baseline,&path,&expected,versions)?;audit["path"]=json!(relative(&root,&path)?);checkpoints.push(audit);
            }
        }
        require(child.wait()?.code()==Some(33),"SDK replacement actual debug exit")?;
        let text=fs::read_to_string(&prepared.serial).map_err(|e|e.to_string())?;markers(&text,commands)?;
        let final_path=directory.join("after.raw");fs::copy(disk,&final_path).map_err(|e|e.to_string())?;
        let mut final_audit=snapshot(baseline,&final_path,&original(),5)?;final_audit["path"]=json!(relative(&root,&final_path)?);
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
    let saved = paths::existing(&run.root, baseline)?;
    audit::inspect(&saved)?;
    let baseline_hash = file_hash(&saved)?;
    let directory = paths::fresh_directory(
        &run.root,
        &format!("out/storage/{}-checked-replace", run.id),
    )?;
    let disk = directory.join("data.raw");
    fs::copy(&saved, &disk).map_err(|e| e.to_string())?;
    let mut boots = Vec::new();
    let mut final_hash = None;
    for first in [true, false] {
        let mut child = Run::new(run.root.clone())?;
        let result = boot(&mut child, image, &disk, &saved, first);
        child.finish(result.as_ref().err().map(String::as_str))?;
        let mut record = result?;
        let hash = record["final"]["sha256"]
            .as_str()
            .ok_or("SDK replacement final hash")?
            .to_owned();
        if let Some(expected) = &final_hash {
            require(
                *expected == hash,
                "SDK replacement separate read-only boot changed disk",
            )?;
        } else {
            final_hash = Some(hash);
        }
        record["report"] = json!({"path":relative(&run.root,&child.directory.join("report.json"))?,"sha256":file_hash(&child.directory.join("report.json"))?});
        boots.push(record);
    }
    require(
        file_hash(&saved)? == baseline_hash,
        "SDK replacement mutated source baseline",
    )?;
    let suite = json!({"schema_version":1,"record_kind":"sdk_checked_replace_suite","outcome":"Passed","verification_state":"guest_tested","baseline":{"path":baseline,"sha256":baseline_hash},"image":image,"boots":boots,"owner_disk_used":false,"scope":"Bounded4096 stream replacement, real private staging/conflicting writer/raw foreign tokens; first-boot raw short-payload audit, maximum restore and second read-only boot; not full filesystem/Notepad fidelity"});
    write_json(&run.directory.join("sdk-checked-replace.json"), &suite)?;
    run.record_guest(&suite)?;
    run.check("p12-sdk-checked-replace", suite);
    Ok(())
}
