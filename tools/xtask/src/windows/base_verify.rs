//! P10 adds measured Windows base execution to every retained P00–P09 scenario.
//! This validator reopens raw bytes and process logs; status labels are not proof.
use crate::{
    evidence::{Run, file_hash, relative},
    paths,
    scenarios::Scenario,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};

pub fn cases() -> [Scenario; 2] {
    ["windows-base", "windows-reopen"].map(|mode| Scenario {
        id: mode,
        mode,
        profile: super::PROFILE,
        expected: crate::emulator::Outcome::Passed,
        markers: vec!["KERNEL_DIAGNOSTIC PASS", "WINDOWS_BASE_ADAPTER_PASS"],
    })
}
pub fn clone_saved(run: &Run, source: &str) -> Result<String, String> {
    let source = paths::existing(&run.root, source)?;
    if crate::services::inspect(&source)?["catalog"]["state"] != "saved" {
        return Err("P10 requires the qualified P08 saved baseline".into());
    }
    let directory = paths::fresh_directory(&run.root, &format!("out/storage/{}-p10", run.id))?;
    for name in ["initial.raw", "data.raw"] {
        fs::copy(&source, directory.join(name)).map_err(|e| e.to_string())?;
    }
    relative(&run.root, &directory.join("data.raw"))
}
pub fn disk_evidence(root: &Path, id: &str) -> Result<Value, String> {
    let mut result = json!({"schema_version":1,"record_kind":"windows_base_disk","root_run":id});
    for (key, name) in [("initial", "initial.raw"), ("final", "data.raw")] {
        let path = format!("out/storage/{id}-p10/{name}");
        result[key] = json!({"path":path,"sha256":file_hash(&paths::existing(root, &path)?)?});
    }
    Ok(result)
}
fn row<'a>(serial: &'a str, prefix: &str) -> Option<&'a str> {
    let mut rows = serial.lines().filter(|line| line.starts_with(prefix));
    let row = rows.next()?;
    rows.next().is_none().then_some(row)
}
fn fields<'a>(line: &'a str, prefix: &str, names: &[&str]) -> Option<Vec<&'a str>> {
    let words: Vec<_> = line.strip_prefix(prefix)?.split(' ').collect();
    if words.len() != names.len() {
        return None;
    }
    words
        .into_iter()
        .zip(names)
        .map(|(word, name)| word.strip_prefix(name))
        .collect()
}
fn number(text: &str) -> Option<u64> {
    let n = text.parse::<u64>().ok()?;
    (n.to_string() == text).then_some(n)
}
fn serial_pid(serial: &str) -> Option<u64> {
    if [
        "DOOR_P08 USER_FAULT",
        "DOOR_P09 FAILURE",
        "DOOR_P09 LOAD_FAIL",
        "DOOR_P10 FAILURE",
    ]
    .iter()
    .any(|bad| serial.contains(bad))
    {
        return None;
    }
    let execution = fields(
        row(serial, "DOOR_P09 EXECUTION ")?,
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
    if pid == 0 || execution[1..3] != ["7", "5"] || execution[5..] != ["0x71002000", "0x71008000"] {
        return None;
    }
    for pointer in &execution[3..5] {
        let value = u64::from_str_radix(pointer.strip_prefix("0x")?, 16).ok()?;
        if value < 0x10000 || format!("{value:#x}") != *pointer {
            return None;
        }
    }
    if row(
        serial,
        &format!("DOOR_USER pid={pid} WINDOWS_BASE_ADAPTER_PASS"),
    )? != format!(
        "DOOR_USER pid={pid} WINDOWS_BASE_ADAPTER_PASS copy=1 atomic=1 limits=1 gs=1 fs=1"
    ) {
        return None;
    }
    let mut console = String::new();
    for line in serial
        .lines()
        .filter(|line| line.starts_with("DOOR_CONSOLE "))
    {
        let (count, quoted) = line
            .strip_prefix("DOOR_CONSOLE bytes=")?
            .split_once(" text=")?;
        let text: String = serde_json::from_str(quoted).ok()?;
        if number(count)? != text.len() as u64 {
            return None;
        }
        console.push_str(&text);
    }
    if console != super::base::OUTPUT {
        return None;
    }
    let mut manifests = BTreeSet::new();
    let mut pids = BTreeSet::new();
    let mut init = 0;
    for line in serial
        .lines()
        .filter(|line| line.starts_with("DOOR_P08 PROCESS "))
    {
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
        let manifest = number(p[0])?;
        let process = number(p[1])?;
        if process == 0 || !manifests.insert(manifest) || !pids.insert(process) {
            return None;
        }
        match manifest {
            1 if p[2..] == ["1", "1", "0", "0", "80"] => init = process,
            2..=4
                if number(p[2])? == manifest
                    && number(p[3])? == manifest - 1
                    && p[4..] == ["0", "0", "0"] => {}
            7 if process == pid
                && p[2..4] == ["5", "1001"]
                && number(p[4])? > 0
                && number(p[5])? > 0
                && p[6] == "74" => {}
            _ => return None,
        }
    }
    if manifests != BTreeSet::from([1, 2, 3, 4, 7]) || init == 0 {
        return None;
    }
    let application = format!("DOOR_USER pid={init} DOOR_P10 APPLICATION PASS pid={pid} exit=74");
    if row(
        serial,
        &format!("DOOR_USER pid={init} DOOR_P10 APPLICATION"),
    )? != application
    {
        return None;
    }
    if row(serial, "DOOR_P08 CLEANUP ")?
        != format!(
            "DOOR_P08 CLEANUP PASS processes=5 threads={} leaked=0",
            super::base::EXPECTED_THREADS
        )
    {
        return None;
    }
    if row(serial, "DOOR_P10 EXCEPTION ")?
        != format!("DOOR_P10 EXCEPTION pid={pid} vector=14 code=0xc0000005 address=0x50000000")
    {
        return None;
    }
    let continuation = fields(
        row(serial, "DOOR_P10 CONTINUE ")?,
        "DOOR_P10 CONTINUE ",
        &["pid=", "rip="],
    )?;
    if number(continuation[0])? != pid
        || u64::from_str_radix(continuation[1].strip_prefix("0x")?, 16).ok()? < 0x10000
    {
        return None;
    }
    let rollback = fields(
        row(serial, "DOOR_P10 MODULE_ROLLBACK ")?,
        "DOOR_P10 MODULE_ROLLBACK PASS ",
        &["checkpoints=", "mappings=", "tls="],
    )?;
    if number(rollback[0])? == 0 || rollback[1..] != ["0", "0"] {
        return None;
    }
    for (case, reason) in [
        ("truncated", "Image(Truncated)"),
        ("bad_machine", "Image(Unsupported)"),
        ("missing_module", "MissingModule"),
        ("missing_symbol", "MissingSymbol"),
        ("collision", "AddressCollision"),
        ("missing_relocation", "RelocationRequired"),
        ("writable_code", "Image(Permissions)"),
    ] {
        if row(serial, &format!("DOOR_P09 REJECT case={case} "))?
            != format!("DOOR_P09 REJECT case={case} reason={reason} baseline=1")
        {
            return None;
        }
    }
    if serial
        .lines()
        .filter(|line| line.starts_with("DOOR_P09 REJECT "))
        .count()
        != 7
    {
        return None;
    }
    if row(serial, &format!("DOOR_USER pid={init} DOOR_P08 SUPERVISOR"))?
        != format!(
            "DOOR_USER pid={init} DOOR_P08 SUPERVISOR PASS services=3 apps=1 restart=0 sessions=1 shutdown=1"
        )
    {
        return None;
    }
    for manifest in 2..=4 {
        let process = serial
            .lines()
            .find(|line| line.starts_with(&format!("DOOR_P08 PROCESS manifest={manifest} ")))?;
        let ready_pid = process.split_once(" pid=")?.1.split(' ').next()?;
        if row(
            serial,
            &format!("DOOR_USER pid={init} DOOR_P08 READY manifest={manifest} "),
        )? != format!(
            "DOOR_USER pid={init} DOOR_P08 READY manifest={manifest} pid={ready_pid} instance={manifest}"
        ) {
            return None;
        }
    }
    let device = fields(
        row(serial, "DOOR_P08 DEVICE ")?,
        "DOOR_P08 DEVICE PASS ",
        &[
            "reads=",
            "writes=",
            "flushes=",
            "sampled_remaining=",
            "reclaimed=",
        ],
    )?;
    if number(device[0])? == 0 || number(device[3])? == 0 || device[4] != "8" {
        return None;
    }
    number(device[1])?;
    number(device[2])?;
    Some(pid)
}
pub fn serial_matches(serial: &str) -> bool {
    serial_pid(serial).is_some()
}
fn binding(root: &Path, binding: &Value) -> Result<(), String> {
    let path = paths::existing(root, binding["path"].as_str().ok_or("P10 bound path")?)?;
    if file_hash(&path)? != binding["sha256"] {
        return Err("P10 bound bytes changed".into());
    }
    Ok(())
}
fn command<'a>(report: &'a Value, id: &Value) -> Result<&'a Value, String> {
    let found: Vec<_> = report["commands"]
        .as_array()
        .ok_or("P10 commands")?
        .iter()
        .filter(|c| c["id"] == *id)
        .collect();
    if found.len() != 1 {
        return Err("P10 command missing/duplicated".into());
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
        && c["spawn_error"].is_null()
        && c["cleanup_error"].is_null()
}
fn artifact<'a>(pass: &'a Value, name: &str) -> Result<&'a Value, String> {
    let found: Vec<_> = pass
        .as_array()
        .ok_or("P10 pass")?
        .iter()
        .filter(|a| a["name"] == name)
        .collect();
    if found.len() != 1 {
        return Err(format!("P10 artifact missing/duplicated {name}"));
    }
    Ok(found[0])
}
fn disk_validate(
    root: &Path,
    report: &Value,
    suite: &Value,
    guests: &[Value],
) -> Result<(), String> {
    let id = report["id"].as_str().ok_or("P10 run id")?;
    let record = disk_evidence(root, id)?;
    if suite["windows_base"] != record
        || record["initial"]["sha256"] != suite["services"]["saved"]["sha256"]
        || guests.len() != 2
    {
        return Err("P10 baseline/guest disk chain differs".into());
    }
    let mut before = crate::services::inspect(&paths::existing(
        root,
        record["initial"]["path"].as_str().unwrap(),
    )?)?;
    let mut digest = record["initial"]["sha256"].clone();
    for (guest, mode) in guests.iter().zip(["windows-base", "windows-reopen"]) {
        let token = guest["run_token"].as_str().ok_or("P10 guest token")?;
        let path = format!("out/qemu/{token}/data-after.raw");
        let after = crate::services::inspect_windows(&paths::existing(root, &path)?)?;
        let after_hash = file_hash(&paths::existing(root, &path)?)?;
        if guest["scenario"] != mode
            || guest["data_disk"]["path"] != record["final"]["path"]
            || guest["data_disk"]["before"] != before
            || guest["data_disk"]["after"]["path"] != path
            || guest["data_disk"]["after"]["audit"] != after
            || guest["data_disk"]["after"]["sha256"] != after_hash
            || !guest["data_disk"]["audit_error"].is_null()
        {
            return Err("P10 raw before/after audit differs".into());
        }
        if mode == "windows-reopen" && (digest != after_hash || before != after) {
            return Err("P10 fresh-process reopen mutated persistent bytes".into());
        }
        digest = json!(after_hash);
        before = after;
    }
    if digest != record["final"]["sha256"] {
        return Err("P10 final disk differs from retained reopen".into());
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
    disk_validate(root, report, suite, guests)?;
    if suite["base_reference_comparison"]
        != json!({"state":"passed","same_binary":true,"output":super::base::OUTPUT,"exit":74})
    {
        return Err("P10 same-binary comparison missing".into());
    }
    validate_artifacts(root, report, passes)?;
    validate_reference(root, report, passes)?;
    let contract = module_contract(root, &passes[0])?;
    for (guest, mode) in guests.iter().zip(["windows-base", "windows-reopen"]) {
        let serial = fs::read_to_string(paths::existing(
            root,
            guest["serial"]["path"].as_str().ok_or("P10 serial")?,
        )?)
        .map_err(|e| e.to_string())?;
        let pid = serial_pid(&serial).ok_or("P10 guest semantics incomplete")?;
        for expected in [
            format!(
                "DOOR_P09 EXECUTION pid={pid} manifest=7 modules=5 entry={:#x} executable={:#x} fs=0x71002000 gs=0x71008000",
                contract["bootstrap"].as_u64().unwrap(),
                contract["entry"].as_u64().unwrap()
            ),
            format!(
                "DOOR_P09 VM PASS rollback={} permissions=1 guards=1",
                contract["initial_checkpoints"].as_u64().unwrap()
            ),
            format!(
                "DOOR_P10 MODULE_ROLLBACK PASS checkpoints={} mappings=0 tls=0",
                contract["dynamic_checkpoints"].as_u64().unwrap()
            ),
        ] {
            if !serial.lines().any(|line| line == expected) {
                return Err("P10 actual mapping/rollback contract differs".into());
            }
        }
        let mut module_count = 0;
        for (index, (name, base)) in [
            ("windows-base", 0x142000000u64),
            ("ntdll", 0x180000000),
            ("kernel32", 0x182000000),
            ("lifecycle", 0x184000000),
            ("msvcrt", 0x186000000),
        ]
        .into_iter()
        .enumerate()
        {
            let a = artifact(&passes[0], name)?;
            let expected = format!(
                "DOOR_P09 MODULE pid={pid} index={index} base={base:#x} preferred={:#x} bytes={} sha256={}",
                a["pe"]["preferred_base"]
                    .as_u64()
                    .ok_or("P10 preferred base")?,
                a["artifact"]["file_bytes"]
                    .as_u64()
                    .ok_or("P10 module bytes")?,
                a["artifact"]["sha256"].as_str().ok_or("P10 module hash")?
            );
            if row(
                &serial,
                &format!("DOOR_P09 MODULE pid={pid} index={index} "),
            ) != Some(expected.as_str())
            {
                return Err("P10 guest module identity differs".into());
            }
            module_count += 1;
        }
        if serial
            .lines()
            .filter(|line| line.starts_with("DOOR_P09 MODULE "))
            .count()
            != module_count
        {
            return Err("P10 extra module records".into());
        }
        if mode == "windows-reopen" {
            let values = fields(
                row(&serial, "DOOR_P08 DEVICE ").ok_or("P10 device")?,
                "DOOR_P08 DEVICE PASS ",
                &[
                    "reads=",
                    "writes=",
                    "flushes=",
                    "sampled_remaining=",
                    "reclaimed=",
                ],
            )
            .ok_or("P10 device fields")?;
            if values[1..3] != ["0", "0"] {
                return Err("P10 reopen performed disk mutation".into());
            }
        }
    }
    Ok(())
}

fn validate_artifacts(root: &Path, report: &Value, passes: &[Value]) -> Result<(), String> {
    if passes.len() != 2 {
        return Err("P10 requires two independent builds".into());
    }
    let sources = report["source_snapshot"]["files"]
        .as_array()
        .ok_or("P10 source identity")?;
    for path in [
        "profiles/build/initfs-p10.json",
        "specs/abi/windows-base-p10.md",
        "specs/abi/windows-io-waits-p10.md",
        "tests/fixtures/windows-base/main.c",
        "tests/fixtures/windows-base/lifecycle.c",
        "tests/fixtures/windows-base/compiler-tls.c",
        "tests/fixtures/windows-base/io_waits.c",
        "tests/fixtures/windows-base/crt.c",
        "tests/fixtures/windows-base/faults.asm",
        "compat/dlls/ntdll/src/files.rs",
        "compat/dlls/ntdll/src/waits.rs",
        "compat/dlls/ntdll/src/loader.rs",
        "compat/dlls/msvcrt/src/lib.rs",
    ] {
        if sources
            .iter()
            .filter(|s| s["path"] == path && s["sha256"].as_str().is_some_and(|v| v.len() == 64))
            .count()
            != 1
        {
            return Err(format!("P10 source binding missing {path}"));
        }
    }
    for pass in passes {
        let archive_bytes = fs::read(paths::existing(
            root,
            pass[2]["path"].as_str().ok_or("P10 initfs path")?,
        )?)
        .map_err(|e| e.to_string())?;
        let archive = door_initfs::Archive::parse(&archive_bytes)
            .map_err(|e| format!("P10 archive {e:?}"))?;
        for name in ["msvcrt", "lifecycle", "windows-base"] {
            let a = artifact(pass, name)?;
            let path = paths::existing(root, a["path"].as_str().ok_or("P10 artifact path")?)?;
            let class = if name == "windows-base" {
                "windows-fixture"
            } else {
                "windows-dll"
            };
            let extension = if name == "windows-base" { "exe" } else { "dll" };
            let destination = format!("windows/{name}.{extension}");
            let target = paths::existing(root, pass[2]["path"].as_str().unwrap())?
                .parent()
                .ok_or("P10 build target")?
                .to_path_buf();
            let expected_path = if name == "msvcrt" {
                target.join("x86_64-pc-windows-msvc/release/msvcrt.dll")
            } else {
                target.join(format!("{name}.{extension}"))
            };
            if !path.starts_with(root.join("out/builds"))
                || path != expected_path
                || a["build_inputs"] != report["source_snapshot"]
                || a["artifact"] != crate::artifact::inspect(&path, class)?
                || a["pe"] != super::pe::inspect(&path)?
                || a["initfs_destination"] != destination
            {
                return Err("P10 artifact source/inspection differs".into());
            }
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            if archive.file(&destination) != Some(bytes.as_slice()) {
                return Err("P10 staged bytes differ".into());
            }
            binding(root, &a["link_map"])?;
            if a["link_map"]["path"] != relative(root, &target.join(format!("{name}.map")))? {
                return Err("P10 map layout differs".into());
            }
            if name == "msvcrt" {
                binding(root, &a["import_library"])?;
                if a["import_library"]["path"]
                    != relative(root, &target.join("windows-imports/msvcrt.lib"))?
                {
                    return Err("P10 CRT import library layout differs".into());
                }
                let target = path.ancestors().nth(3).ok_or("P10 CRT target")?;
                let c = command(report, &a["build_command"])?;
                let argv: Vec<_> = std::iter::once("cargo".to_owned())
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
                    return Err("P10 original CRT build differs".into());
                }
                let imports: Vec<_> = a["pe"]["imports"]
                    .as_array()
                    .ok_or("CRT imports")?
                    .iter()
                    .map(|m| m["module"].as_str().unwrap_or("").to_ascii_lowercase())
                    .collect();
                if imports != ["ntdll.dll"] || a["pe"]["entry_rva"] != 0 {
                    return Err("P10 CRT host dependency".into());
                }
            } else {
                validate_fixture_commands(root, report, a, &path)?;
            }
        }
    }
    for name in ["msvcrt", "lifecycle", "windows-base"] {
        let a = artifact(&passes[0], name)?;
        let b = artifact(&passes[1], name)?;
        if a["path"] == b["path"] || a["artifact"]["sha256"] != b["artifact"]["sha256"] {
            return Err("P10 builds do not independently reproduce".into());
        }
    }
    Ok(())
}
fn validate_fixture_commands(
    root: &Path,
    report: &Value,
    a: &Value,
    path: &Path,
) -> Result<(), String> {
    let profile = super::build::toolchain(root)?;
    if a["toolchain"] != profile || a["headers"].as_array().is_none_or(Vec::is_empty) {
        return Err("P10 SDK identity missing".into());
    }
    for header in a["headers"].as_array().unwrap() {
        if file_hash(Path::new(header["path"].as_str().ok_or("P10 SDK header")?))?
            != header["sha256"]
        {
            return Err("P10 SDK header changed".into());
        }
    }
    let target = path.parent().ok_or("P10 fixture directory")?;
    let planned = fixture_commands(root, target, a["name"].as_str().ok_or("P10 fixture name")?)?;
    let ids = a["commands"]["all"]
        .as_array()
        .ok_or("P10 complete command identities")?;
    if ids.len() != planned.len() {
        return Err("P10 complete build command count differs".into());
    }
    let mut headers = Vec::new();
    for (id, argv) in ids.iter().zip(planned) {
        let c = command(report, id)?;
        if c["argv"] != json!(argv) || c["required"] != true || !successful(c, 0, 120000) {
            return Err("P10 exact independent compiler/linker command missing".into());
        }
        for key in ["stdout", "stderr"] {
            binding(root, &c[key])?;
        }
        if argv[0] == super::build::executable(&profile, "cl.exe")? {
            let stdout = fs::read_to_string(root.join(c["stdout"]["path"].as_str().unwrap()))
                .map_err(|e| e.to_string())?;
            let stderr = fs::read_to_string(root.join(c["stderr"]["path"].as_str().unwrap()))
                .map_err(|e| e.to_string())?;
            headers.extend(super::build::included_headers(&stdout, &stderr)?);
        }
    }
    headers.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    headers.dedup_by(|a, b| a["path"] == b["path"]);
    if a["headers"] != json!(headers) {
        return Err("P10 compiler-observed SDK header inventory differs".into());
    }
    Ok(())
}
fn fixture_commands(root: &Path, target: &Path, name: &str) -> Result<Vec<Vec<String>>, String> {
    let profile = super::build::toolchain(root)?;
    let dll = name == "lifecycle";
    if !dll && name != "windows-base" {
        return Err("P10 independent fixture name".into());
    }
    let source = super::native_path(&root.join("tests/fixtures/windows-base").join(if dll {
        "lifecycle.c"
    } else {
        "main.c"
    }))?;
    let (compile, link) = super::build::fixture_args(root, target, &profile)?;
    let compile: Vec<_> = compile
        .into_iter()
        .map(|arg| {
            if arg.ends_with("tests/fixtures/windows-x64/main.c") {
                source.clone()
            } else {
                arg.replace("windows-basic", name)
            }
        })
        .collect();
    let extension = if dll { "dll" } else { "exe" };
    let mut link: Vec<_> = link
        .into_iter()
        .filter(|arg| !arg.ends_with("windows-abi.obj"))
        .map(|arg| {
            arg.replace("windows-basic", name)
                .replace(&format!("{name}.exe"), &format!("{name}.{extension}"))
        })
        .collect();
    let mut planned = Vec::new();
    if dll {
        let tls: Vec<_> = compile
            .iter()
            .map(|arg| {
                arg.replace("lifecycle.c", "compiler-tls.c")
                    .replace("lifecycle.obj", "compiler-tls.obj")
            })
            .collect();
        planned.push(("cl.exe", tls));
        link.push(super::native_path(&target.join("compiler-tls.obj"))?);
        link.push("/INCLUDE:_tls_used".into());
        for arg in &mut link {
            if arg == "/ENTRY:entry" {
                *arg = "/ENTRY:DllMain".into();
            }
            if arg == "/BASE:0x140000000" {
                *arg = "/BASE:0x183000000".into();
            }
        }
        link.push("/DLL".into());
        link.push(format!(
            "/IMPLIB:{}",
            super::native_path(&target.join("lifecycle.lib"))?
        ));
    } else {
        link.push(super::native_path(
            &target.join("windows-imports/msvcrt.lib"),
        )?);
        let crt: Vec<_> = compile
            .iter()
            .map(|arg| {
                arg.replace("windows-base/main.c", "windows-base/crt.c")
                    .replace("windows-base.obj", "crt.obj")
            })
            .chain(["/Oi-".to_owned()])
            .collect();
        planned.push(("cl.exe", crt));
        link.push(super::native_path(&target.join("crt.obj"))?);
        let io: Vec<_> = compile
            .iter()
            .map(|arg| {
                arg.replace("windows-base/main.c", "windows-base/io_waits.c")
                    .replace("windows-base.obj", "io-waits.obj")
            })
            .collect();
        planned.push(("cl.exe", io));
        link.push(super::native_path(&target.join("io-waits.obj"))?);
        planned.push((
            "ml64.exe",
            vec![
                "/nologo".into(),
                "/c".into(),
                "/W3".into(),
                "/WX".into(),
                format!(
                    "/Fo{}",
                    super::native_path(&target.join("windows-faults.obj"))?
                ),
                super::native_path(&root.join("tests/fixtures/windows-base/faults.asm"))?,
            ],
        ));
        link.push(super::native_path(&target.join("windows-faults.obj"))?);
    }
    planned.push(("cl.exe", compile));
    planned.push(("link.exe", link));
    planned
        .into_iter()
        .map(|(tool, args)| {
            Ok(std::iter::once(super::build::executable(&profile, tool)?)
                .chain(args)
                .collect())
        })
        .collect()
}
fn module_contract(root: &Path, pass: &Value) -> Result<Value, String> {
    let names = ["windows-base", "ntdll", "kernel32", "lifecycle", "msvcrt"];
    let bases = [
        0x142000000,
        0x180000000,
        0x182000000,
        0x184000000,
        0x186000000,
    ];
    let mut files = Vec::new();
    for name in names {
        files.push(
            fs::read(paths::existing(
                root,
                artifact(pass, name)?["path"]
                    .as_str()
                    .ok_or("P10 planned artifact")?,
            )?)
            .map_err(|e| e.to_string())?,
        );
    }
    let module_names = [
        "windows-base.exe",
        "ntdll.dll",
        "kernel32.dll",
        "lifecycle.dll",
        "msvcrt.dll",
    ];
    let sources: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(index, bytes)| door_pe_loader::ModuleSource {
            name: module_names[index],
            bytes,
            base: door_pe_loader::ImageBase(bases[index]),
        })
        .collect();
    let plan =
        door_pe_loader::BundlePlan::with_policy(&sources, door_pe_loader::LoadPolicy::DllCallbacks)
            .map_err(|e| format!("P10 actual PE plan {e:?}"))?;
    let mut initial = 15;
    let mut dynamic = 0;
    plan.visit_runs(|run| {
        let mapped = run.permissions != door_pe_loader::Permissions::None;
        if run.module == 3 {
            dynamic += if mapped { 3 } else { 1 };
        } else {
            initial += if mapped { 4 } else { 2 };
        }
        Ok::<_, String>(())
    })
    .map_err(|e| format!("P10 runs {e:?}"))?;
    plan.visit_fixups(|fixup| {
        if fixup.module == 3 {
            dynamic += 1;
        }
        Ok::<_, String>(())
    })
    .map_err(|e| format!("P10 fixups {e:?}"))?;
    let bootstrap = plan
        .resolve_executable_export(1, door_pe_loader::Symbol::Name("DoorProcessBootstrap"))
        .map_err(|e| format!("P10 bootstrap {e:?}"))?
        .0;
    Ok(
        json!({"entry":plan.entry().0,"bootstrap":bootstrap,"initial_checkpoints":initial,"dynamic_checkpoints":dynamic}),
    )
}
fn validate_reference(root: &Path, report: &Value, passes: &[Value]) -> Result<(), String> {
    let checks: Vec<_> = report["checks"]
        .as_array()
        .ok_or("P10 checks")?
        .iter()
        .filter(|c| c["id"] == "p10-windows-reference" && c["outcome"] == "passed")
        .collect();
    if checks.len() != 1 {
        return Err("P10 reference missing/duplicated".into());
    }
    let reference = &checks[0]["details"];
    let fixture = artifact(&passes[0], "windows-base")?;
    let path = paths::existing(root, fixture["path"].as_str().ok_or("P10 reference path")?)?;
    if reference["record_kind"] != "windows_base_reference"
        || reference["outcome"] != "passed"
        || reference["sha256"] != fixture["artifact"]["sha256"]
        || reference["stdout"] != super::base::OUTPUT
        || reference["exit"] != 74
        || reference["reopen"]["stdout"] != super::base::OUTPUT
        || reference["reopen"]["exit"] != 74
    {
        return Err("P10 same-binary Windows reference differs".into());
    }
    for (id, args) in [
        (&reference["command"], Vec::<String>::new()),
        (&reference["reopen"]["command"], vec!["--reopen".to_owned()]),
    ] {
        let c = command(report, id)?;
        let argv: Vec<_> = std::iter::once(path.to_str().ok_or("P10 executable")?.to_owned())
            .chain(args)
            .collect();
        if c["argv"] != json!(argv)
            || c["cwd"] != json!(path.parent().ok_or("P10 CWD")?)
            || !successful(c, 74, 10000)
        {
            return Err("P10 measured reference process/CWD differs".into());
        }
        for (key, expected) in [("stdout", super::base::OUTPUT), ("stderr", "")] {
            binding(root, &c[key])?;
            if fs::read_to_string(paths::existing(
                root,
                c[key]["path"].as_str().ok_or("P10 reference stream")?,
            )?)
            .map_err(|e| e.to_string())?
                != expected
            {
                return Err("P10 actual reference stream differs".into());
            }
        }
    }
    let identity = command(report, &reference["identity_command"])?;
    if identity["required"] != true || !successful(identity, 0, 120000) {
        return Err("P10 reference OS observation missing".into());
    }
    binding(root, &identity["stdout"])?;
    let text = fs::read_to_string(paths::existing(
        root,
        identity["stdout"]["path"].as_str().ok_or("P10 OS stream")?,
    )?)
    .map_err(|e| e.to_string())?;
    let observed: Value = serde_json::from_str(text.trim()).map_err(|e| e.to_string())?;
    if reference["identity"] != observed
        || !observed["caption"]
            .as_str()
            .is_some_and(|v| v.contains("Windows"))
        || observed["build"].as_str().is_none_or(str::is_empty)
    {
        return Err("P10 reference OS differs".into());
    }
    let expected: Vec<_> = (0..1024).map(|i| (i % 512 + 1 + i / 512) as u8).collect();
    binding(root, &reference["data"])?;
    if reference["data"]["bytes"] != 1024
        || fs::read(paths::existing(
            root,
            reference["data"]["path"]
                .as_str()
                .ok_or("P10 reference data")?,
        )?)
        .map_err(|e| e.to_string())?
            != expected
    {
        return Err("P10 independently computed reference data differs".into());
    }
    let expected_artifacts: Vec<_> = ["msvcrt", "lifecycle", "windows-base"]
        .into_iter()
        .map(|name| artifact(&passes[0], name).cloned())
        .collect::<Result<_, _>>()?;
    if reference["artifacts"] != json!(expected_artifacts) {
        return Err("P10 reference artifact inventory differs".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid_serial() -> String {
        let mut lines = vec![
            "DOOR_P09 EXECUTION pid=11 manifest=7 modules=5 entry=0x180001000 executable=0x142001000 fs=0x71002000 gs=0x71008000".to_owned(),
            "DOOR_USER pid=11 WINDOWS_BASE_ADAPTER_PASS copy=1 atomic=1 limits=1 gs=1 fs=1".into(),
            "DOOR_P10 EXCEPTION pid=11 vector=14 code=0xc0000005 address=0x50000000".into(),
            "DOOR_P10 CONTINUE pid=11 rip=0x142001234".into(),
            "DOOR_P10 MODULE_ROLLBACK PASS checkpoints=40 mappings=0 tls=0".into(),
            format!("DOOR_CONSOLE bytes={} text={:?}",super::super::base::OUTPUT.len(),super::super::base::OUTPUT),
            "DOOR_P08 PROCESS manifest=7 pid=11 instance=5 uid=1001 session=1 epoch=1 exit=74".into(),
            "DOOR_USER pid=7 DOOR_P10 APPLICATION PASS pid=11 exit=74".into(),
            "DOOR_USER pid=7 DOOR_P08 SUPERVISOR PASS services=3 apps=1 restart=0 sessions=1 shutdown=1".into(),
            "DOOR_P08 PROCESS manifest=1 pid=7 instance=1 uid=1 session=0 epoch=0 exit=80".into(),
            format!("DOOR_P08 CLEANUP PASS processes=5 threads={} leaked=0",super::super::base::EXPECTED_THREADS),
            "DOOR_P08 DEVICE PASS reads=123 writes=0 flushes=0 sampled_remaining=1 reclaimed=8".into(),
        ];
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
        lines.join("\n")
    }
    #[test]
    fn p10_retains_every_prior_scenario_and_adds_ordered_separate_boots() {
        let previous = crate::scenarios::cases("P09").unwrap();
        let current = crate::scenarios::cases("P10").unwrap();
        assert_eq!((previous.len(), current.len()), (40, 42));
        for (a, b) in previous.iter().zip(&current) {
            assert_eq!(
                (a.id, a.mode, a.profile, &a.expected, &a.markers),
                (b.id, b.mode, b.profile, &b.expected, &b.markers)
            );
        }
        assert_eq!(current[40].mode, "windows-base");
        assert_eq!(current[41].mode, "windows-reopen");
        assert_eq!(
            crate::scenarios::check_id("P10").unwrap(),
            "p10-windows-base"
        );
        assert!(crate::scenarios::serial_matches(
            "P10",
            &current[40].markers,
            &valid_serial()
        ));
    }
    #[test]
    fn base_serial_rejects_missing_duplicate_and_conflicting_subsystem_records() {
        let serial = valid_serial();
        assert!(serial_matches(&serial));
        for line in serial.lines() {
            assert!(
                !serial_matches(&serial.replace(line, "")),
                "accepted removed {line}"
            );
            assert!(
                !serial_matches(&format!("{serial}\n{line}")),
                "accepted duplicate {line}"
            );
        }
        for (from, to) in [
            ("manifest=7 modules=5", "manifest=6 modules=5"),
            ("uid=1001", "uid=0"),
            ("exit=74", "exit=0"),
            ("gs=0x71008000", "gs=0x71002000"),
            ("atomic=1", "atomic=0"),
            ("leaked=0", "leaked=1"),
            ("reclaimed=8", "reclaimed=7"),
            ("mappings=0", "mappings=1"),
            ("tls=0", "tls=1"),
            ("checkpoints=40", "checkpoints=0"),
            ("pid=11", "pid=011"),
            ("vector=14", "vector=6"),
            ("baseline=1", "baseline=0"),
            ("restart=0", "restart=1"),
        ] {
            assert!(
                !serial_matches(&serial.replace(from, to)),
                "accepted {from}->{to}"
            );
        }
    }
    #[test]
    fn process_policy_requires_measured_success_and_finite_timeout() {
        let good = json!({"outcome":"passed","exit_code":74,"expected_exit_code":74,"timeout_ms":10000,"timed_out":false,"output_limit_exceeded":false,"cleanup_error":null,"spawn_error":null});
        assert!(successful(&good, 74, 10000));
        for (key, value) in [
            ("exit_code", json!(0)),
            ("expected_exit_code", json!(0)),
            ("timeout_ms", json!(120000)),
            ("timed_out", json!(true)),
            ("output_limit_exceeded", json!(true)),
            ("cleanup_error", json!("failed")),
            ("spawn_error", json!("failed")),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(!successful(&bad, 74, 10000));
        }
    }
}
