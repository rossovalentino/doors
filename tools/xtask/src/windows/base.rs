//! Independent P10 DLL/process fixture build and measured Windows reference.
use crate::{
    artifact,
    evidence::{Run, file_hash, relative},
};
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};
pub const EXPECTED_THREADS: u64 = 38;

pub const OUTPUT: &str = "WINDOWS_BASE_DLL attach=1\r\nWINDOWS_BASE_DLL detach=1 unloaded=1\r\nWINDOWS_BASE_DLL attach=1\r\nUNWIND_LOOKUP\r\nUNWIND_FRAME\r\nUNWIND_PASS\r\nWINDOWS_BASE_OK lifecycle=1 tls=1 threads=1 memory=1 files=1 exception=1\r\nWINDOWS_BASE_DLL detach=1 quiesced=1\r\n";

pub fn build(run: &mut Run, target: &Path) -> Result<Vec<Value>, String> {
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    let profile = super::build::toolchain(&run.root)?;
    let mut artifacts = super::dlls::build_crt(run, target)?;
    for (name, source, dll) in [
        ("lifecycle", "lifecycle.c", true),
        ("windows-base", "main.c", false),
    ] {
        let command_start = run.commands().len();
        let mut headers = Vec::new();
        let mut assembly_id = Value::Null;
        let (compile, link) = super::build::fixture_args(&run.root, target, &profile)?;
        let source =
            super::native_path(&run.root.join("tests/fixtures/windows-base").join(source))?;
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
        if dll {
            let tls_compile: Vec<_> = compile
                .iter()
                .map(|arg| {
                    arg.replace("lifecycle.c", "compiler-tls.c")
                        .replace("lifecycle.obj", "compiler-tls.obj")
                })
                .collect();
            let observed = run.required(
                &super::build::executable(&profile, "cl.exe")?,
                &tls_compile.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
            headers.extend(super::build::included_headers(
                &observed.stdout,
                &observed.stderr,
            )?);
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
            let crt_compile: Vec<_> = compile
                .iter()
                .map(|arg| {
                    arg.replace("windows-base/main.c", "windows-base/crt.c")
                        .replace("windows-base.obj", "crt.obj")
                })
                .chain(["/Oi-".to_owned()])
                .collect();
            let observed = run.required(
                &super::build::executable(&profile, "cl.exe")?,
                &crt_compile.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
            headers.extend(super::build::included_headers(
                &observed.stdout,
                &observed.stderr,
            )?);
            link.push(super::native_path(&target.join("crt.obj"))?);
            let io_compile: Vec<_> = compile
                .iter()
                .map(|arg| {
                    arg.replace("windows-base/main.c", "windows-base/io_waits.c")
                        .replace("windows-base.obj", "io-waits.obj")
                })
                .collect();
            let observed = run.required(
                &super::build::executable(&profile, "cl.exe")?,
                &io_compile.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
            headers.extend(super::build::included_headers(
                &observed.stdout,
                &observed.stderr,
            )?);
            link.push(super::native_path(&target.join("io-waits.obj"))?);
            let assembly = [
                "/nologo".to_owned(),
                "/c".into(),
                "/W3".into(),
                "/WX".into(),
                format!(
                    "/Fo{}",
                    super::native_path(&target.join("windows-faults.obj"))?
                ),
                super::native_path(&run.root.join("tests/fixtures/windows-base/faults.asm"))?,
            ];
            run.required(
                &super::build::executable(&profile, "ml64.exe")?,
                &assembly.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
            assembly_id = run.commands().last().ok_or("P10 assembly command")?["id"].clone();
            link.push(super::native_path(&target.join("windows-faults.obj"))?);
        }
        let compile_id = run.commands().len();
        let observed = run.required(
            &super::build::executable(&profile, "cl.exe")?,
            &compile.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        headers.extend(super::build::included_headers(
            &observed.stdout,
            &observed.stderr,
        )?);
        headers.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        headers.dedup_by(|a, b| a["path"] == b["path"]);
        let link_id = run.commands().len();
        run.required(
            &super::build::executable(&profile, "link.exe")?,
            &link.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        let path = target.join(format!("{name}.{extension}"));
        let map = target.join(format!("{name}.map"));
        let class = if dll {
            "windows-dll"
        } else {
            "windows-fixture"
        };
        let all_commands: Vec<_> = run.commands()[command_start..]
            .iter()
            .map(|command| command["id"].clone())
            .collect();
        artifacts.push(json!({"name":name,"path":relative(&run.root,&path)?,"artifact":artifact::inspect(&path,class)?,"pe":super::pe::inspect(&path)?,"initfs_destination":format!("windows/{name}.{extension}"),"build_inputs":run.build_inputs(),"toolchain":profile,"headers":headers,"commands":{"all":all_commands,"compile":run.commands()[compile_id]["id"],"assembly":assembly_id,"link":run.commands()[link_id]["id"]},"link_map":{"path":relative(&run.root,&map)?,"sha256":file_hash(&map)?}}));
    }
    Ok(artifacts)
}

pub fn reference(run: &mut Run, artifacts: &[Value]) -> Result<Value, String> {
    let fixture = artifacts
        .iter()
        .find(|a| a["name"] == "windows-base")
        .ok_or("P10 fixture missing")?;
    let path = run
        .root
        .join(fixture["path"].as_str().ok_or("P10 fixture path")?);
    let before = file_hash(&path)?;
    let identity = run.required("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", "$referenceOS = Get-CimInstance Win32_OperatingSystem; [pscustomobject]@{caption=$referenceOS.Caption;version=$referenceOS.Version;build=$referenceOS.BuildNumber;architecture=$referenceOS.OSArchitecture} | ConvertTo-Json -Compress"])?;
    let identity_command = run
        .commands()
        .last()
        .ok_or("P10 Windows identity command")?["id"]
        .clone();
    let identity: Value =
        serde_json::from_str(identity.stdout.trim()).map_err(|e| e.to_string())?;
    // The fixture first proves its data file absent; a reused development
    // target directory (`Run::dev_cache`) still holds the last run's.
    let stale = path
        .parent()
        .ok_or("P10 data directory")?
        .join("p10-data-\u{03bb}.bin");
    if stale.exists() {
        std::fs::remove_file(&stale).map_err(|e| format!("{}: {e}", stale.display()))?;
    }
    let output = run.watched_in(
        path.parent().ok_or("P10 fixture working directory")?,
        path.to_str().ok_or("P10 executable")?,
        &[],
        Duration::from_secs(10),
        74,
    )?;
    if !output.success
        || output.stdout != OUTPUT
        || !output.stderr.is_empty()
        || before != file_hash(&path)?
    {
        return Err(format!(
            "P10 Windows reference failed exit={:?} output={:?}",
            output.exit_code, output.stdout
        ));
    }
    let save_command = run.commands().last().ok_or("P10 save command")?["id"].clone();
    let data_path = path
        .parent()
        .ok_or("P10 data directory")?
        .join("p10-data-λ.bin");
    let data = fs::read(&data_path).map_err(|e| e.to_string())?;
    let expected: Vec<_> = (0..1024)
        .map(|index| (index % 512 + 1 + index / 512) as u8)
        .collect();
    if data != expected {
        return Err("P10 reference file bytes differ from independent computation".into());
    }
    let data_hash = file_hash(&data_path)?;
    let reopen = run.watched_in(
        path.parent().ok_or("P10 fixture directory")?,
        path.to_str().ok_or("P10 executable")?,
        &["--reopen"],
        Duration::from_secs(10),
        74,
    )?;
    if !reopen.success
        || reopen.stdout != OUTPUT
        || !reopen.stderr.is_empty()
        || file_hash(&data_path)? != data_hash
    {
        return Err("P10 independent reopen failed or altered saved bytes".into());
    }
    for artifact in artifacts {
        if file_hash(
            &run.root
                .join(artifact["path"].as_str().ok_or("P10 artifact")?),
        )? != artifact["artifact"]["sha256"]
        {
            return Err("P10 reference binary bytes changed".into());
        }
    }
    Ok(
        json!({"record_kind":"windows_base_reference","outcome":"passed","identity":identity,"identity_command":identity_command,"sha256":before,"stdout":output.stdout,"exit":74,"command":save_command,"reopen":{"command":run.commands().last().ok_or("P10 reference command")?["id"],"stdout":reopen.stdout,"exit":74},"data":{"path":relative(&run.root,&data_path)?,"sha256":data_hash,"bytes":1024},"artifacts":artifacts}),
    )
}

pub fn standalone(run: &mut Run) -> Result<(), String> {
    let target = run.root.join(format!("out/builds/{}/windows-base", run.id));
    let artifacts = build(run, &target)?;
    let record = reference(run, &artifacts)?;
    crate::evidence::write_json(&run.directory.join("windows-base-reference.json"), &record)?;
    run.check("p10-windows-reference", record);
    Ok(())
}
