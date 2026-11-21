//! The P09 gate binds ordinary Windows execution to the exact guest image.
use crate::{evidence::file_hash, paths, scenarios::Scenario};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};

pub fn case() -> Scenario {
    Scenario {
        id: super::MODE,
        mode: super::MODE,
        profile: super::PROFILE,
        expected: crate::emulator::Outcome::Passed,
        markers: vec!["KERNEL_DIAGNOSTIC PASS", "DOOR_P09 EXECUTION"],
    }
}
fn rows<'a>(serial: &'a str, prefix: &str) -> Vec<&'a str> {
    serial
        .lines()
        .filter(|line| line.starts_with(prefix))
        .collect()
}
fn number(text: &str) -> Option<u64> {
    let value = text.parse::<u64>().ok()?;
    (value.to_string() == text).then_some(value)
}
fn fields<'a>(line: &'a str, prefix: &str, names: &[&str]) -> Option<Vec<&'a str>> {
    let words: Vec<_> = line.strip_prefix(prefix)?.split(' ').collect();
    if words.len() != names.len() {
        return None;
    }
    words
        .iter()
        .zip(names)
        .map(|(word, name)| word.strip_prefix(name))
        .collect()
}
fn one<'a>(serial: &'a str, prefix: &str) -> Option<&'a str> {
    let found = rows(serial, prefix);
    (found.len() == 1).then(|| found[0])
}
fn serial_result(serial: &str) -> Option<u64> {
    if serial.contains("DOOR_P08 USER_FAULT") || serial.contains("DOOR_P09 FAILURE") {
        return None;
    }
    let execution = fields(
        one(serial, "DOOR_P09 EXECUTION")?,
        "DOOR_P09 EXECUTION ",
        &[
            "pid=",
            "manifest=",
            "modules=",
            "entry=",
            "executable=",
            "fs=",
            "gs=",
        ],
    )?;
    let pid = number(execution[0])?;
    if pid == 0 || execution[1..3] != ["6", "3"] || execution[5..] != ["0x71002000", "0x71008000"] {
        return None;
    }
    for pointer in &execution[3..5] {
        let address = u64::from_str_radix(pointer.strip_prefix("0x")?, 16).ok()?;
        if format!("{address:#x}") != *pointer || address < 0x10000 {
            return None;
        }
    }
    let expected_rejects = [
        ("truncated", "Image(Truncated)"),
        ("bad_machine", "Image(Unsupported)"),
        ("missing_module", "MissingModule"),
        ("missing_symbol", "MissingSymbol"),
        ("collision", "AddressCollision"),
        ("missing_relocation", "RelocationRequired"),
        ("writable_code", "Image(Permissions)"),
    ];
    let rejects = rows(serial, "DOOR_P09 REJECT");
    if rejects.len() != expected_rejects.len()
        || expected_rejects.iter().any(|(case, reason)| {
            !rejects.contains(
                &format!("DOOR_P09 REJECT case={case} reason={reason} baseline=1").as_str(),
            )
        })
    {
        return None;
    }
    let vm = fields(
        one(serial, "DOOR_P09 VM")?,
        "DOOR_P09 VM PASS ",
        &["rollback=", "permissions=", "guards="],
    )?;
    if number(vm[0])? < 15 || vm[1..] != ["1", "1"] {
        return None;
    }
    let modules = rows(serial, "DOOR_P09 MODULE");
    if modules.len() != 3 {
        return None;
    }
    for (index, line) in modules.iter().enumerate() {
        let m = fields(
            line,
            "DOOR_P09 MODULE ",
            &["pid=", "index=", "base=", "preferred=", "bytes=", "sha256="],
        )?;
        if number(m[0])? != pid
            || number(m[1])? != index as u64
            || number(m[4])? == 0
            || m[5].len() != 64
            || !m[5]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return None;
        }
    }
    let processes = rows(serial, "DOOR_P08 PROCESS");
    if processes.len() != 5 {
        return None;
    }
    let mut pids = BTreeSet::new();
    let mut instances = BTreeSet::new();
    let mut manifests = BTreeSet::new();
    let mut init = 0;
    for line in processes {
        let p = fields(
            line,
            "DOOR_P08 PROCESS ",
            &[
                "manifest=",
                "pid=",
                "instance=",
                "uid=",
                "session=",
                "epoch=",
                "exit=",
            ],
        )?;
        let m = number(p[0])?;
        let process = number(p[1])?;
        let instance = number(p[2])?;
        if instance != (if m == 6 { 5 } else { m })
            || process == 0
            || instance == 0
            || !pids.insert(process)
            || !instances.insert(instance)
            || !manifests.insert(m)
        {
            return None;
        }
        match m {
            1 => {
                init = process;
                if p[3..] != ["1", "0", "0", "80"] {
                    return None;
                }
            }
            2..=4 => {
                if number(p[3])? != m - 1 || p[4..] != ["0", "0", "0"] {
                    return None;
                }
            }
            6 => {
                if process != pid
                    || p[3] != "1001"
                    || number(p[4])? == 0
                    || number(p[5])? == 0
                    || p[6] != "73"
                {
                    return None;
                }
            }
            _ => return None,
        }
    }
    for (prefix, line) in [
        (
            "DOOR_P08 CLEANUP",
            "DOOR_P08 CLEANUP PASS processes=5 threads=5 leaked=0".to_owned(),
        ),
        (
            "DOOR_CONSOLE",
            format!(
                "DOOR_CONSOLE bytes={} text={:?}",
                super::OUTPUT.len(),
                super::OUTPUT
            ),
        ),
    ] {
        if one(serial, prefix)? != line {
            return None;
        }
    }
    let receipts = [
        format!(
            "DOOR_USER pid={pid} WINDOWS_ADAPTER_PASS copy=1 atomic=1 limits=1 thread_denied=1 heap_denied=1 raw_denied=1 gs=1 fs=1"
        ),
        format!("DOOR_USER pid={init} DOOR_P09 APPLICATION PASS pid={pid} exit=73"),
        format!(
            "DOOR_USER pid={init} DOOR_P08 SUPERVISOR PASS services=3 apps=1 restart=0 sessions=1 shutdown=1"
        ),
    ];
    for (receipt, marker) in receipts.into_iter().zip([
        "WINDOWS_ADAPTER_PASS",
        "DOOR_P09 APPLICATION",
        "DOOR_P08 SUPERVISOR",
    ]) {
        if serial
            .lines()
            .filter(|line| line.starts_with("DOOR_USER ") && line.contains(marker))
            .collect::<Vec<_>>()
            != [receipt.as_str()]
        {
            return None;
        }
    }
    let readiness = rows(serial, &format!("DOOR_USER pid={init} DOOR_P08 READY"));
    if readiness.len() != 3 {
        return None;
    }
    for manifest in 2..=4 {
        let process = rows(serial, "DOOR_P08 PROCESS")
            .into_iter()
            .find(|line| line.starts_with(&format!("DOOR_P08 PROCESS manifest={manifest} ")))?;
        let pid = process.split_once(" pid=")?.1.split(' ').next()?;
        let expected = format!(
            "DOOR_USER pid={init} DOOR_P08 READY manifest={manifest} pid={pid} instance={manifest}"
        );
        if !readiness.contains(&expected.as_str()) {
            return None;
        }
    }
    let device = fields(
        one(serial, "DOOR_P08 DEVICE")?,
        "DOOR_P08 DEVICE PASS ",
        &[
            "reads=",
            "writes=",
            "flushes=",
            "sampled_remaining=",
            "reclaimed=",
        ],
    )?;
    if number(device[0])? == 0
        || device[1..3] != ["0", "0"]
        || number(device[3])? == 0
        || device[4] != "8"
    {
        return None;
    }
    Some(pid)
}
pub fn serial_matches(serial: &str) -> bool {
    serial_result(serial).is_some()
}
fn command<'a>(report: &'a Value, id: &Value) -> Result<&'a Value, String> {
    let found: Vec<_> = report["commands"]
        .as_array()
        .ok_or("Windows commands")?
        .iter()
        .filter(|c| c["id"] == *id)
        .collect();
    if found.len() != 1 {
        return Err("Windows command missing/duplicated".into());
    }
    Ok(found[0])
}
fn successful(c: &Value, exit: i32, timeout: u64) -> bool {
    c["outcome"] == "passed"
        && c["exit_code"] == exit
        && c["expected_exit_code"] == exit
        && c["timeout_ms"] == timeout
        && c["timed_out"] == false
        && c["output_limit_exceeded"] == false
        && c["cleanup_error"].is_null()
        && c["spawn_error"].is_null()
}
fn binding(root: &Path, b: &Value) -> Result<(), String> {
    let path = paths::existing(root, b["path"].as_str().ok_or("Windows binding path")?)?;
    if file_hash(&path)? != b["sha256"] {
        return Err("Windows bound bytes changed".into());
    }
    Ok(())
}
pub fn validate(
    root: &Path,
    report: &Value,
    suite: &Value,
    passes: &[Value],
    guests: &[Value],
) -> Result<(), String> {
    super::session::validate(
        root,
        report["id"].as_str().ok_or("Windows root run")?,
        suite,
        guests,
    )?;
    let expected_comparison =
        json!({"state":"passed","same_binary":true,"output":super::OUTPUT,"exit":super::EXIT});
    if suite["reference_comparison"] != expected_comparison {
        return Err("Windows reference comparison missing".into());
    }
    let sources = report["source_snapshot"]["files"]
        .as_array()
        .ok_or("Windows source identity")?;
    for path in [
        super::build::SOURCE,
        super::build::ASSEMBLY,
        super::build::PROFILE,
        super::PROFILE,
        "profiles/build/initfs-p09.json",
        "specs/abi/windows-x64-p09.md",
        "compat/abi/src/lib.rs",
        "compat/pe/src/lib.rs",
        "compat/dlls/ntdll/src/lib.rs",
        "compat/dlls/kernel32/src/lib.rs",
    ] {
        if sources
            .iter()
            .filter(|f| f["path"] == path && f["sha256"].as_str().is_some_and(|s| s.len() == 64))
            .count()
            != 1
        {
            return Err(format!("Windows source identity missing {path}"));
        }
    }
    let profile = super::build::toolchain(root)?;
    let mut first = Vec::new();
    for (pass_index, pass) in passes.iter().enumerate() {
        let artifacts = pass.as_array().ok_or("Windows build pass")?;
        let archive_bytes = fs::read(paths::existing(
            root,
            pass[2]["path"].as_str().ok_or("Windows initfs")?,
        )?)
        .map_err(|e| e.to_string())?;
        let archive = door_initfs::Archive::parse(&archive_bytes)
            .map_err(|e| format!("Windows initfs {e:?}"))?;
        for name in ["ntdll", "kernel32", super::FIXTURE] {
            let matching: Vec<_> = artifacts.iter().filter(|a| a["name"] == name).collect();
            if matching.len() != 1 {
                return Err("Windows artifact missing/duplicated".into());
            }
            let a = matching[0];
            let path = paths::existing(root, a["path"].as_str().ok_or("Windows artifact path")?)?;
            let fixture = name == super::FIXTURE;
            let target = if fixture {
                path.parent()
            } else {
                path.ancestors().nth(3)
            }
            .ok_or("Windows target")?;
            let expected = if fixture {
                target.join("windows-basic.exe")
            } else {
                target.join(format!("x86_64-pc-windows-msvc/release/{name}.dll"))
            };
            let destination = if fixture {
                "windows/windows-basic.exe".into()
            } else {
                format!("windows/{}", super::dlls::file_name(name))
            };
            if path != expected
                || !path.starts_with(root.join("out/builds"))
                || a["build_inputs"] != report["source_snapshot"]
                || a["initfs_destination"] != destination
                || crate::artifact::inspect(
                    &path,
                    if fixture {
                        "windows-fixture"
                    } else {
                        "windows-dll"
                    },
                )? != a["artifact"]
                || super::pe::inspect(&path)? != a["pe"]
            {
                return Err("Windows artifact identity/inspection differs".into());
            }
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            if archive.file(&destination) != Some(bytes.as_slice()) {
                return Err("Windows initfs bytes differ".into());
            }
            binding(root, &a["link_map"])?;
            if a["link_map"]["path"]
                != crate::evidence::relative(root, &target.join(format!("{name}.map")))?
            {
                return Err("Windows link map path differs".into());
            }
            if fixture {
                super::pe::fixture(&a["pe"])?;
                if a["toolchain"] != profile || a["headers"].as_array().is_none_or(Vec::is_empty) {
                    return Err("Windows fixture SDK binding missing".into());
                }
                for header in a["headers"].as_array().unwrap() {
                    if file_hash(Path::new(header["path"].as_str().ok_or("SDK header")?))?
                        != header["sha256"]
                    {
                        return Err("SDK header changed".into());
                    }
                }
                let (compile, link) = super::build::fixture_args(root, target, &profile)?;
                for (key, tool, args) in [
                    ("compile", "cl.exe", compile),
                    ("link", "link.exe", link),
                    (
                        "assembly",
                        "ml64.exe",
                        super::build::assembler_args(root, target)?,
                    ),
                ] {
                    let c = command(report, &a["commands"][key])?;
                    let argv: Vec<_> = std::iter::once(super::build::executable(&profile, tool)?)
                        .chain(args)
                        .collect();
                    if c["argv"] != json!(argv)
                        || c["required"] != true
                        || !successful(c, 0, 120000)
                    {
                        return Err("Windows independent compile/link command differs".into());
                    }
                }
                if a["bundle_plan"] != super::pe::bundle(root, artifacts)? {
                    return Err("Windows mapped bundle differs".into());
                }
            } else {
                binding(root, &a["import_library"])?;
                if a["import_library"]["path"]
                    != crate::evidence::relative(
                        root,
                        &target.join(format!("windows-imports/{name}.lib")),
                    )?
                {
                    return Err("original import library path differs".into());
                }
                let c = command(report, &a["build_command"])?;
                let argv: Vec<_> = std::iter::once("cargo".into())
                    .chain(super::dlls::argv(root, target, name)?)
                    .collect();
                if c["argv"] != json!(argv)
                    || c["required"] != true
                    || !successful(c, 0, super::dlls::BUILD_BOUND.as_millis() as u64)
                    || c["environment_overrides"]
                        != json!([
                            [
                                "CARGO_ENCODED_RUSTFLAGS",
                                super::dlls::flags(root, target, name)?
                            ],
                            ["CARGO_PROFILE_RELEASE_LTO", "false"]
                        ])
                {
                    return Err("original DLL command/flags differs".into());
                }
                super::dlls::check_graph(name, &a["pe"])
                    .map_err(|e| format!("original DLL host dependency/entry point: {e}"))?;
            }
            if pass_index == 0 {
                first.push(a.clone());
            }
        }
    }
    let checks = report["checks"]
        .as_array()
        .ok_or("Windows reference checks")?;
    let references: Vec<_> = checks
        .iter()
        .filter(|c| c["id"] == "p09-windows-reference" && c["outcome"] == "passed")
        .collect();
    if references.len() != 1 {
        return Err("Windows reference missing/duplicated".into());
    }
    let reference = &references[0]["details"];
    let fixture = &first[2];
    if reference["path"] != fixture["path"]
        || reference["sha256"] != fixture["artifact"]["sha256"]
        || reference["expected_exit"] != super::EXIT
        || reference["stdout"] != super::OUTPUT
        || reference["stderr"] != ""
        || reference["outcome"] != "passed"
        || reference["binary_modified"] != false
    {
        return Err("same-binary Windows reference differs".into());
    }
    let c = command(report, &reference["process_command"])?;
    let path = paths::existing(root, reference["path"].as_str().ok_or("reference path")?)?;
    if c["argv"] != json!([path.to_str().ok_or("reference executable")?])
        || !successful(c, super::EXIT, 10000)
    {
        return Err("actual Windows reference process differs".into());
    }
    for (key, expected) in [("stdout", super::OUTPUT), ("stderr", "")] {
        binding(root, &c[key])?;
        if fs::read_to_string(paths::existing(
            root,
            c[key]["path"].as_str().ok_or("reference stream")?,
        )?)
        .map_err(|e| e.to_string())?
            != expected
        {
            return Err("actual reference stream differs".into());
        }
    }
    let identity = command(report, &reference["identity_command"])?;
    if !successful(identity, 0, 120000) || identity["required"] != true {
        return Err("reference OS identity failed".into());
    }
    let identity_text = fs::read_to_string(paths::existing(
        root,
        identity["stdout"]["path"]
            .as_str()
            .ok_or("OS identity stream")?,
    )?)
    .map_err(|e| e.to_string())?;
    let actual_identity: Value =
        serde_json::from_str(identity_text.trim()).map_err(|e| e.to_string())?;
    if reference["identity"] != actual_identity
        || !actual_identity["caption"]
            .as_str()
            .is_some_and(|s| s.contains("Windows"))
        || actual_identity["build"].as_str().is_none_or(str::is_empty)
    {
        return Err("reference OS identity differs".into());
    }
    let serial = fs::read_to_string(paths::existing(
        root,
        guests[0]["serial"]["path"]
            .as_str()
            .ok_or("Windows serial")?,
    )?)
    .map_err(|e| e.to_string())?;
    let pid = serial_result(&serial).ok_or("Windows serial semantics")?;
    let bundle = &fixture["bundle_plan"];
    for (index, a) in [&first[2], &first[0], &first[1]].iter().enumerate() {
        let line = format!(
            "DOOR_P09 MODULE pid={pid} index={index} base={:#x} preferred={:#x} bytes={} sha256={}",
            bundle["bases"][index].as_u64().ok_or("base")?,
            a["pe"]["preferred_base"].as_u64().ok_or("preferred base")?,
            a["artifact"]["file_bytes"]
                .as_u64()
                .ok_or("artifact bytes")?,
            a["artifact"]["sha256"].as_str().ok_or("artifact hash")?
        );
        if !serial.lines().any(|s| s == line) {
            return Err("guest PE module bytes/base differ".into());
        }
    }
    let execution = format!(
        "DOOR_P09 EXECUTION pid={pid} manifest=6 modules=3 entry={:#x} executable={:#x} fs=0x71002000 gs=0x71008000",
        bundle["bootstrap"].as_u64().ok_or("bootstrap")?,
        bundle["entry"].as_u64().ok_or("entry")?
    );
    let runs = bundle["runs"].as_array().ok_or("PE runs")?;
    let checkpoints = 15
        + runs
            .iter()
            .map(|r| if r["permissions"] == "None" { 2 } else { 4 })
            .sum::<u64>();
    if !serial.lines().any(|s| s == execution)
        || !serial
            .lines()
            .any(|s| s == format!("DOOR_P09 VM PASS rollback={checkpoints} permissions=1 guards=1"))
        || bundle["bases"][0] == fixture["pe"]["preferred_base"]
    {
        return Err("guest mapping/rollback/rebase differs from actual PE plan".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid() -> String {
        let mut lines = Vec::new();
        for (case, reason) in [
            ("truncated", "Image(Truncated)"),
            ("bad_machine", "Image(Unsupported)"),
            ("missing_module", "MissingModule"),
            ("missing_symbol", "MissingSymbol"),
            ("collision", "AddressCollision"),
            ("missing_relocation", "RelocationRequired"),
            ("writable_code", "Image(Permissions)"),
        ] {
            lines.push(format!(
                "DOOR_P09 REJECT case={case} reason={reason} baseline=1"
            ));
        }
        lines.push("DOOR_P09 VM PASS rollback=79 permissions=1 guards=1".into());
        for i in 0..3 {
            lines.push(format!("DOOR_P09 MODULE pid=11 index={i} base=0x142000000 preferred=0x140000000 bytes=5120 sha256={}","a".repeat(64)));
        }
        lines.push("DOOR_P09 EXECUTION pid=11 manifest=6 modules=3 entry=0x180001070 executable=0x142001000 fs=0x71002000 gs=0x71008000".into());
        lines.push("DOOR_USER pid=11 WINDOWS_ADAPTER_PASS copy=1 atomic=1 limits=1 thread_denied=1 heap_denied=1 raw_denied=1 gs=1 fs=1".into());
        lines.push(format!(
            "DOOR_CONSOLE bytes={} text={:?}",
            super::super::OUTPUT.len(),
            super::super::OUTPUT
        ));
        lines.push(
            "DOOR_P08 PROCESS manifest=6 pid=11 instance=5 uid=1001 session=1 epoch=1 exit=73"
                .into(),
        );
        lines.push("DOOR_USER pid=7 DOOR_P09 APPLICATION PASS pid=11 exit=73".into());
        for m in 2..=4 {
            lines.push(format!(
                "DOOR_P08 PROCESS manifest={m} pid={} instance={m} uid={} session=0 epoch=0 exit=0",
                m + 6,
                m - 1
            ));
            lines.push(format!(
                "DOOR_USER pid=7 DOOR_P08 READY manifest={m} pid={} instance={m}",
                m + 6
            ));
        }
        lines.push("DOOR_USER pid=7 DOOR_P08 SUPERVISOR PASS services=3 apps=1 restart=0 sessions=1 shutdown=1".into());
        lines.push(
            "DOOR_P08 PROCESS manifest=1 pid=7 instance=1 uid=1 session=0 epoch=0 exit=80".into(),
        );
        lines.push("DOOR_P08 CLEANUP PASS processes=5 threads=5 leaked=0".into());
        lines.push(
            "DOOR_P08 DEVICE PASS reads=336 writes=0 flushes=0 sampled_remaining=56896 reclaimed=8"
                .into(),
        );
        lines.join("\n")
    }
    #[test]
    fn p09_requires_attribution_errors_atomic_copy_and_complete_retirement() {
        let serial = valid();
        assert!(serial_matches(&serial));
        for line in serial.lines() {
            assert!(
                !serial_matches(&serial.replace(line, "")),
                "accepted removal: {line}"
            );
            assert!(
                !serial_matches(&format!("{serial}\n{line}")),
                "accepted duplicate: {line}"
            );
        }
        for (from, to) in [
            ("pid=11 WINDOWS", "pid=10 WINDOWS"),
            ("exit=73", "exit=0"),
            ("uid=1001", "uid=0"),
            ("permissions=1", "permissions=0"),
            ("guards=1", "guards=0"),
            ("atomic=1", "atomic=0"),
            ("bytes=100", "bytes=99"),
            ("writes=0", "writes=1"),
            ("reclaimed=8", "reclaimed=7"),
            ("leaked=0", "leaked=1"),
            ("gs=0x71008000", "gs=0x71002000"),
            ("instance=5", "instance=1"),
            ("reason=MissingSymbol", "reason=MissingModule"),
            ("pid=11", "pid=011"),
        ] {
            assert!(
                !serial_matches(&serial.replace(from, to)),
                "accepted {from} -> {to}"
            );
        }
    }
    #[test]
    fn p09_preserves_every_prior_case_and_adds_exact_windows_case() {
        let old = crate::scenarios::cases("P08").unwrap();
        let new = crate::scenarios::cases("P09").unwrap();
        assert_eq!(old.len(), 39);
        assert_eq!(new.len(), 40);
        for (a, b) in old.iter().zip(&new) {
            assert_eq!(
                (a.id, a.mode, a.profile, &a.markers, &a.expected),
                (b.id, b.mode, b.profile, &b.markers, &b.expected)
            );
        }
        assert_eq!(new.last().unwrap().id, super::super::MODE);
        assert!(crate::scenarios::serial_matches(
            "P09",
            &new.last().unwrap().markers,
            &valid()
        ));
    }
}
