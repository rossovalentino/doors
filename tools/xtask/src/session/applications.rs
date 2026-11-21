//! Reproducible third-party source ports, with original Win32 platform glue.
use super::*;
pub fn build(run: &mut Run, target: &Path) -> Result<Value, String> {
    let provenance = image::read_json(&run.root.join("third_party/apps/tinybasic/UPSTREAM.json"))?;
    for entry in provenance["files"].as_array().ok_or("upstream files")? {
        if file_hash(
            &run.root
                .join(entry["path"].as_str().ok_or("upstream path")?),
        )? != entry["sha256"]
        {
            return Err("Pinned Tiny Basic source changed".into());
        }
    }
    let profile = crate::windows::build::toolchain(&run.root)?;
    let msvc = profile["msvc_root"].as_str().ok_or("MSVC path")?;
    let compiler = format!("{msvc}/bin/Hostx64/x64/cl.exe");
    let linker = format!("{msvc}/bin/Hostx64/x64/link.exe");
    let sdk = profile["sdk_root"].as_str().ok_or("SDK path")?;
    let version = profile["sdk_version"].as_str().ok_or("SDK version")?;
    let native = crate::windows::native_path;
    let mut headers = Vec::new();
    for (name, source, strict) in [
        ("interpreter", "third_party/apps/tinybasic/int-stc.c", false),
        (
            "runtime",
            "tests/windows-fixtures/tinybasic/runtime.c",
            true,
        ),
    ] {
        let mut args = vec![
            "/nologo".into(),
            "/c".into(),
            "/TC".into(),
            "/std:c11".into(),
            "/O1".into(),
            "/Oi-".into(),
            "/GS-".into(),
            "/Zl".into(),
            "/X".into(),
            "/Brepro".into(),
            "/showIncludes".into(),
            format!(
                "/I{}",
                native(&run.root.join("tests/windows-fixtures/tinybasic/include"))?
            ),
            format!("/I{msvc}/include"),
            format!("/I{sdk}/Include/{version}/ucrt"),
            format!("/Fo{}", native(&target.join(format!("{name}.obj")))?),
            native(&run.root.join(source))?,
        ];
        args.push(if strict { "/W4" } else { "/W3" }.into());
        if strict {
            args.push("/WX".into());
        }
        let observed = run.required(
            &compiler,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        headers.extend(crate::windows::build::included_headers(
            &observed.stdout,
            &observed.stderr,
        )?);
    }
    let path = target.join("tinybasic.exe");
    let args = vec![
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
        "/BASE:0x140000000".into(),
        "/STACK:65536,4096".into(),
        format!("/OUT:{}", native(&path)?),
        native(&target.join("interpreter.obj"))?,
        native(&target.join("runtime.obj"))?,
        format!("{sdk}/Lib/{version}/um/x64/kernel32.lib"),
    ];
    run.required(
        &linker,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let result = json!({"name":"tinybasic","path":relative(&run.root,&path)?,"sha256":file_hash(&path)?,"provenance":provenance,"headers":headers,"pe":crate::windows::pe::inspect(&path)?,"runtime":"unchanged third-party C interpreter; original Win32 C platform source port"});
    run.check("tinybasic-build", result.clone());
    let program = target.join("reference.bas");
    fs::write(
        &program,
        b"10 a=0\n20 for i=1 to 10\n30 a=a+i\n40 next i\n50 print a\n60 print 2^10\n70 bye\n",
    )
    .map_err(|e| e.to_string())?;
    let observed = run.watched(
        path.to_str().ok_or("application path")?,
        &[program.to_str().ok_or("BASIC reference path")?],
        Duration::from_secs(10),
        0,
        &[],
    )?;
    if !observed.success || observed.stdout != "55\n1024\n" || !observed.stderr.is_empty() {
        return Err(format!(
            "Tiny Basic Windows reference mismatch: {:?}",
            observed.stdout.chars().take(512).collect::<String>()
        ));
    }
    run.check("tinybasic-windows-reference",json!({"executable_sha256":file_hash(&path)?,"program_sha256":file_hash(&program)?,"stdout":observed.stdout,"exit":observed.exit_code}));
    Ok(result)
}
