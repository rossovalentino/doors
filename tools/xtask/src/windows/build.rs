use crate::{
    artifact,
    evidence::{Run, file_hash, relative, write_json},
    image, paths,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path, time::Duration};
pub const PROFILE: &str = "profiles/build/windows-x64.json";
pub const SOURCE: &str = "tests/fixtures/windows-x64/main.c";
pub const ASSEMBLY: &str = "tests/fixtures/windows-x64/abi.asm";

pub fn assembler_args(root: &Path, target: &Path) -> Result<Vec<String>, String> {
    Ok(vec![
        "/nologo".into(),
        "/c".into(),
        "/W3".into(),
        "/WX".into(),
        format!(
            "/Fo{}",
            super::native_path(&target.join("windows-abi.obj"))?
        ),
        super::native_path(&root.join(ASSEMBLY))?,
    ])
}

pub fn toolchain(root: &Path) -> Result<Value, String> {
    let profile = image::read_json(&paths::existing(root, PROFILE)?)?;
    if profile["schema_version"] != 1 || profile["record_kind"] != "windows_fixture_toolchain" {
        return Err("Windows toolchain profile".into());
    }
    for input in profile["files"]
        .as_array()
        .ok_or("Windows toolchain files")?
    {
        let path = Path::new(input["path"].as_str().ok_or("Windows toolchain path")?);
        if !path.is_absolute() || file_hash(path)? != input["sha256"] {
            return Err(format!(
                "Windows toolchain acquisition changed: {}",
                path.display()
            ));
        }
    }
    Ok(profile)
}
pub(super) fn executable(profile: &Value, name: &str) -> Result<String, String> {
    Ok(format!(
        "{}/bin/Hostx64/x64/{name}",
        profile["msvc_root"].as_str().ok_or("MSVC path")?
    ))
}
pub fn fixture_args(
    root: &Path,
    target: &Path,
    profile: &Value,
) -> Result<(Vec<String>, Vec<String>), String> {
    let msvc = profile["msvc_root"].as_str().ok_or("MSVC root")?;
    let sdk = profile["sdk_root"].as_str().ok_or("SDK root")?;
    let version = profile["sdk_version"].as_str().ok_or("SDK version")?;
    let mut compile = vec![
        "/nologo".into(),
        "/c".into(),
        "/TC".into(),
        "/O1".into(),
        "/GS-".into(),
        "/Zl".into(),
        "/W4".into(),
        "/WX".into(),
        "/X".into(),
        "/showIncludes".into(),
        "/Brepro".into(),
        format!("/I{msvc}/include"),
    ];
    for directory in ["um", "shared", "ucrt"] {
        compile.push(format!("/I{sdk}/Include/{version}/{directory}"));
    }
    compile.extend([
        format!(
            "/Fo{}",
            super::native_path(&target.join("windows-basic.obj"))?
        ),
        super::native_path(&root.join(SOURCE))?,
    ]);
    let link = vec![
        "/NOLOGO".into(),
        "/NODEFAULTLIB".into(),
        "/ENTRY:entry".into(),
        "/SUBSYSTEM:CONSOLE,6.02".into(),
        "/MACHINE:X64".into(),
        "/DYNAMICBASE".into(),
        "/FIXED:NO".into(),
        "/NXCOMPAT".into(),
        "/Brepro".into(),
        "/DEBUG:NONE".into(),
        "/INCREMENTAL:NO".into(),
        "/MANIFEST:NO".into(),
        "/OPT:REF".into(),
        "/OPT:ICF".into(),
        "/BASE:0x140000000".into(),
        "/STACK:65536,4096".into(),
        "/HEAP:65536,4096".into(),
        format!(
            "/OUT:{}",
            super::native_path(&target.join("windows-basic.exe"))?
        ),
        format!(
            "/MAP:{}",
            super::native_path(&target.join("windows-basic.map"))?
        ),
        super::native_path(&target.join("windows-basic.obj"))?,
        super::native_path(&target.join("windows-abi.obj"))?,
        format!("{sdk}/Lib/{version}/um/x64/kernel32.lib"),
    ];
    Ok((compile, link))
}
pub fn included_headers(stdout: &str, stderr: &str) -> Result<Vec<Value>, String> {
    let mut headers = BTreeSet::new();
    for line in stdout.lines().chain(stderr.lines()) {
        if let Some(path) = line.trim().strip_prefix("Note: including file:") {
            headers.insert(path.trim().replace('\\', "/"));
        }
    }
    if headers.is_empty() {
        return Err("MSVC did not report included SDK header identities".into());
    }
    headers
        .into_iter()
        .map(|path| Ok(json!({"sha256":file_hash(Path::new(&path))?,"path":path})))
        .collect()
}

pub fn fixture(run: &mut Run, target: &Path) -> Result<Value, String> {
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    let profile = toolchain(&run.root)?;
    let assembly = assembler_args(&run.root, target)?;
    run.required(
        &executable(&profile, "ml64.exe")?,
        &assembly.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let assembly_id = run.commands().last().ok_or("fixture assembler command")?["id"].clone();
    let (compile, link) = fixture_args(&run.root, target, &profile)?;
    let observed = run.required(
        &executable(&profile, "cl.exe")?,
        &compile.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let compile_id = run.commands().last().ok_or("fixture compiler command")?["id"].clone();
    let headers = included_headers(&observed.stdout, &observed.stderr)?;
    run.required(
        &executable(&profile, "link.exe")?,
        &link.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let link_id = run.commands().last().ok_or("fixture linker command")?["id"].clone();
    let path = target.join("windows-basic.exe");
    let metadata = super::pe::inspect(&path)?;
    super::pe::fixture(&metadata)?;
    let inspected = artifact::inspect(&path, "windows-fixture")?;
    let map = target.join("windows-basic.map");
    Ok(
        json!({"name":super::FIXTURE,"path":relative(&run.root,&path)?,"artifact":inspected,"pe":metadata,"build_inputs":run.build_inputs(),"initfs_destination":"windows/windows-basic.exe","toolchain":profile,"headers":headers,"commands":{"compile":compile_id,"assembly":assembly_id,"link":link_id},"link_map":{"path":relative(&run.root,&map)?,"sha256":file_hash(&map)?},"runtime":"independent Windows SDK C/MASM; no CRT"}),
    )
}
pub fn reference(run: &mut Run, fixture: &Value) -> Result<Value, String> {
    let path = paths::existing(
        &run.root,
        fixture["path"].as_str().ok_or("Windows fixture path")?,
    )?;
    let before = file_hash(&path)?;
    if fixture["artifact"]["sha256"] != before {
        return Err("Windows reference binary changed before launch".into());
    }
    let identity=run.required("powershell.exe",&["-NoProfile","-NonInteractive","-Command","$referenceOS = Get-CimInstance Win32_OperatingSystem; [pscustomobject]@{caption=$referenceOS.Caption;version=$referenceOS.Version;build=$referenceOS.BuildNumber;architecture=$referenceOS.OSArchitecture} | ConvertTo-Json -Compress"])?;
    let identity: Value =
        serde_json::from_str(identity.stdout.trim()).map_err(|e| e.to_string())?;
    let identity_command = run
        .commands()
        .last()
        .ok_or("Windows reference identity command")?["id"]
        .clone();
    let observed = run.watched(
        path.to_str().ok_or("Windows reference executable")?,
        &[],
        Duration::from_secs(10),
        super::EXIT,
        &[],
    )?;
    if !observed.success
        || observed.stdout != super::OUTPUT
        || !observed.stderr.is_empty()
        || file_hash(&path)? != before
    {
        return Err(format!(
            "Windows reference output/exit mismatch (exit {:?}): {:?}",
            observed.exit_code, observed.stdout
        ));
    }
    Ok(
        json!({"schema_version":1,"record_kind":"windows_reference","identity":identity,"identity_command":identity_command,"path":relative(&run.root,&path)?,"sha256":before,"expected_exit":super::EXIT,"stdout":super::OUTPUT,"stderr":"","process_command":run.commands().last().ok_or("Windows reference command")?["id"],"outcome":"passed","binary_modified":false}),
    )
}
pub fn standalone(run: &mut Run, include_dlls: bool) -> Result<(), String> {
    let target = paths::fresh_directory(
        &run.root,
        &format!("out/builds/{}/windows-reference", run.id),
    )?;
    let mut passes = Vec::new();
    for index in 0..2 {
        let directory = target.join(format!("pass-{index}"));
        let mut pass = if include_dlls {
            super::dlls::build(run, &directory)?
        } else {
            Vec::new()
        };
        pass.push(fixture(run, &directory)?);
        if include_dlls {
            let bundle = super::pe::bundle(&run.root, &pass)?;
            pass.last_mut().ok_or("Windows fixture")?["bundle_plan"] = bundle;
        }
        passes.push(pass);
    }
    if passes[0]
        .iter()
        .zip(&passes[1])
        .any(|(a, b)| a["artifact"]["sha256"] != b["artifact"]["sha256"])
    {
        return Err("independent Windows fixture rebuild differs".into());
    }
    let reference = reference(run, passes[0].last().ok_or("Windows fixture artifact")?)?;
    let record = json!({"artifacts":passes,"reference":reference,"scope":"Windows reference only; Door guest qualification remains required"});
    write_json(&run.directory.join("windows-reference.json"), &record)?;
    run.check("windows-reference", record);
    Ok(())
}
