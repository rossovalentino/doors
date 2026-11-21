//! P13 Win32 GUI fixtures: F1 self-test, F2 editor (code and resource
//! variants) and F3 hang, compiled twice with the pinned MSVC toolchain and SDK
//! GUI import libraries, their import inventory, and the measured Windows
//! reference run. The same fixture bytes later run in the Door guest
//! (`crate::desktop::win32`).
use crate::{
    artifact,
    evidence::{Run, file_hash, relative, write_json},
    image, paths,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, time::Duration};

pub const DIRECTORY: &str = "tests/fixtures/windows-gui";
/// Published import inventory (P13-A/P13-B check their export coverage here).
pub const INVENTORY: &str = "tests/fixtures/windows-gui/imports.json";
/// Published Windows reference results and metrics for the current fixtures.
pub const REFERENCE: &str = "tests/fixtures/windows-gui/reference.json";
pub const GUI_LIBRARIES: [&str; 4] = ["User32.Lib", "Gdi32.Lib", "ComDlg32.Lib", "ComCtl32.Lib"];
/// The only modules an ordinary GUI fixture may import.
pub const ALLOWED_MODULES: [&str; 5] = [
    "kernel32.dll",
    "user32.dll",
    "gdi32.dll",
    "comdlg32.dll",
    "comctl32.dll",
];

pub struct Fixture {
    pub id: &'static str,
    pub name: &'static str,
    pub source: &'static str,
    pub resources: Option<&'static str>,
    pub defines: &'static [&'static str],
}
pub const FIXTURES: [Fixture; 4] = [
    Fixture {
        id: "F1",
        name: "wingui-selftest",
        source: "selftest.c",
        resources: None,
        defines: &[],
    },
    Fixture {
        id: "F2",
        name: "wingui-editor",
        source: "editor.c",
        resources: None,
        defines: &[],
    },
    Fixture {
        id: "F2-RC",
        name: "wingui-editor-rc",
        source: "editor.c",
        resources: Some("editor.rc"),
        defines: &["/DEDITOR_RESOURCES"],
    },
    Fixture {
        id: "F3",
        name: "wingui-hang",
        source: "hang.c",
        resources: None,
        defines: &[],
    },
];

/// One reference process: fixture, argument, expected exit, results file and
/// the private working directory it shares with related runs.
pub struct Launch {
    pub fixture: &'static str,
    pub argument: &'static str,
    pub exit: i32,
    pub results: &'static str,
    pub directory: &'static str,
    pub timeout_seconds: u64,
}
/// Windows reference order. IsHungAppWindow needs more than five seconds
/// without message retrieval, so F1 and F3 get a 60 s watcher (not 10 s).
pub const LAUNCHES: [Launch; 9] = [
    Launch {
        fixture: "wingui-selftest",
        argument: "",
        exit: 75,
        results: "wingui-selftest.results",
        directory: "f1",
        timeout_seconds: 60,
    },
    Launch {
        fixture: "wingui-selftest",
        argument: "--reopen",
        exit: 76,
        results: "wingui-selftest-reopen.results",
        directory: "f1",
        timeout_seconds: 60,
    },
    Launch {
        fixture: "wingui-selftest",
        argument: "--probe",
        exit: 77,
        results: "wingui-selftest-probe.results",
        directory: "f1-probe",
        timeout_seconds: 60,
    },
    Launch {
        fixture: "wingui-editor",
        argument: "--selftest",
        exit: 80,
        results: "wingui-editor.results",
        directory: "f2",
        timeout_seconds: 60,
    },
    Launch {
        fixture: "wingui-editor",
        argument: "--reopen",
        exit: 81,
        results: "wingui-editor.results",
        directory: "f2",
        timeout_seconds: 60,
    },
    Launch {
        fixture: "wingui-editor-rc",
        argument: "--selftest",
        exit: 80,
        results: "wingui-editor-rc.results",
        directory: "f2-rc",
        timeout_seconds: 60,
    },
    Launch {
        fixture: "wingui-editor-rc",
        argument: "--reopen",
        exit: 81,
        results: "wingui-editor-rc.results",
        directory: "f2-rc",
        timeout_seconds: 60,
    },
    Launch {
        fixture: "wingui-hang",
        argument: "--selftest",
        exit: 82,
        results: "wingui-hang.results",
        directory: "f3",
        timeout_seconds: 60,
    },
    Launch {
        fixture: "wingui-hang",
        argument: "--reject",
        exit: 83,
        results: "wingui-hang.results",
        directory: "f3",
        timeout_seconds: 60,
    },
];
/// Documents the fixtures save, by working directory.
pub const SAVED: [(&str, &str); 3] = [
    ("f1", "wingui-selftest-\u{3bb}.txt"),
    ("f2", "doors-editor-\u{3bb}.txt"),
    ("f2-rc", "doors-editor-\u{3bb}.txt"),
];

fn compile_args(
    root: &Path,
    target: &Path,
    profile: &Value,
    fixture: &Fixture,
) -> Result<Vec<String>, String> {
    let msvc = profile["msvc_root"].as_str().ok_or("MSVC root")?;
    let sdk = profile["sdk_root"].as_str().ok_or("SDK root")?;
    let version = profile["sdk_version"].as_str().ok_or("SDK version")?;
    let mut args: Vec<String> = [
        "/nologo",
        "/c",
        "/TC",
        "/O1",
        "/GS-",
        "/Zl",
        "/W4",
        "/WX",
        "/X",
        "/showIncludes",
        "/Brepro",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(format!("/I{msvc}/include"));
    for directory in ["um", "shared", "ucrt"] {
        args.push(format!("/I{sdk}/Include/{version}/{directory}"));
    }
    args.extend(fixture.defines.iter().map(|d| (*d).to_owned()));
    args.push(format!(
        "/Fo{}",
        super::native_path(&target.join(format!("{}.obj", fixture.name)))?
    ));
    args.push(super::native_path(
        &root.join(DIRECTORY).join(fixture.source),
    )?);
    Ok(args)
}
fn resource_args(
    root: &Path,
    target: &Path,
    profile: &Value,
    fixture: &Fixture,
    rc: &str,
) -> Result<Vec<String>, String> {
    let msvc = profile["msvc_root"].as_str().ok_or("MSVC root")?;
    let sdk = profile["sdk_root"].as_str().ok_or("SDK root")?;
    let version = profile["sdk_version"].as_str().ok_or("SDK version")?;
    Ok(vec![
        "/nologo".into(),
        "/x".into(),
        format!("/I{sdk}/Include/{version}/um"),
        format!("/I{sdk}/Include/{version}/shared"),
        format!("/I{msvc}/include"),
        format!("/I{}", super::native_path(&root.join(DIRECTORY))?),
        format!(
            "/fo{}",
            super::native_path(&target.join(format!("{}.res", fixture.name)))?
        ),
        super::native_path(&root.join(DIRECTORY).join(rc))?,
    ])
}
pub fn link_args(target: &Path, profile: &Value, fixture: &Fixture) -> Result<Vec<String>, String> {
    let sdk = profile["sdk_root"].as_str().ok_or("SDK root")?;
    let version = profile["sdk_version"].as_str().ok_or("SDK version")?;
    let mut args: Vec<String> = [
        "/NOLOGO",
        "/NODEFAULTLIB",
        "/ENTRY:entry",
        "/SUBSYSTEM:WINDOWS,6.02",
        "/MACHINE:X64",
        "/DYNAMICBASE",
        "/FIXED:NO",
        "/NXCOMPAT",
        "/Brepro",
        "/DEBUG:NONE",
        "/INCREMENTAL:NO",
        "/MANIFEST:NO",
        "/OPT:REF",
        "/OPT:ICF",
        "/BASE:0x140000000",
        "/STACK:1048576,65536",
        "/HEAP:65536,4096",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(format!(
        "/OUT:{}",
        super::native_path(&target.join(format!("{}.exe", fixture.name)))?
    ));
    args.push(format!(
        "/MAP:{}",
        super::native_path(&target.join(format!("{}.map", fixture.name)))?
    ));
    args.push(super::native_path(
        &target.join(format!("{}.obj", fixture.name)),
    )?);
    if fixture.resources.is_some() {
        args.push(super::native_path(
            &target.join(format!("{}.res", fixture.name)),
        )?);
    }
    args.push(format!("{sdk}/Lib/{version}/um/x64/kernel32.lib"));
    for library in GUI_LIBRARIES {
        args.push(format!("{sdk}/Lib/{version}/um/x64/{library}"));
    }
    Ok(args)
}

/// Every pinned GUI import library and the resource compiler are part of the
/// verified toolchain acquisition.
pub fn toolchain(root: &Path) -> Result<Value, String> {
    let profile = super::build::toolchain(root)?;
    let files: Vec<_> = profile["files"]
        .as_array()
        .ok_or("Windows toolchain files")?
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();
    let rc = profile["resource_compiler"]
        .as_str()
        .ok_or("pinned resource compiler")?;
    for required in GUI_LIBRARIES.iter().map(|l| format!("/um/x64/{l}")).chain([
        "rc.exe".to_owned(),
        "rcdll.dll".to_owned(),
        "cvtres.exe".to_owned(),
    ]) {
        if !files.iter().any(|f| f.ends_with(&required)) {
            return Err(format!("Windows GUI toolchain does not pin {required}"));
        }
    }
    if !files.contains(&rc) || profile["gui_import_libraries"] != json!(GUI_LIBRARIES) {
        return Err("Windows GUI toolchain profile".into());
    }
    Ok(profile)
}

/// Ordinary imports only: named symbols from the allowed system modules.
pub fn imports(metadata: &Value) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut modules = BTreeMap::new();
    for module in metadata["imports"].as_array().ok_or("fixture imports")? {
        let name = module["module"]
            .as_str()
            .ok_or("fixture import module")?
            .to_ascii_lowercase();
        if !ALLOWED_MODULES.contains(&name.as_str()) {
            return Err(format!("GUI fixture imports unexpected module {name}"));
        }
        let mut symbols = module["symbols"]
            .as_array()
            .ok_or("fixture import symbols")?
            .iter()
            .map(|s| {
                s["name"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or("GUI fixture ordinal imports are unsupported".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        symbols.sort();
        if modules.insert(name.clone(), symbols).is_some() {
            return Err(format!("GUI fixture imports {name} twice"));
        }
    }
    if !modules.contains_key("kernel32.dll") || !modules.contains_key("user32.dll") {
        return Err("GUI fixture must import kernel32.dll and user32.dll".into());
    }
    Ok(modules)
}

fn build_one(
    run: &mut Run,
    target: &Path,
    profile: &Value,
    fixture: &Fixture,
) -> Result<Value, String> {
    let start = run.commands().len();
    let compile = compile_args(&run.root, target, profile, fixture)?;
    let observed = run.required(
        &super::build::executable(profile, "cl.exe")?,
        &compile.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let mut headers = super::build::included_headers(&observed.stdout, &observed.stderr)?;
    let mut resources = Value::Null;
    if let Some(rc) = fixture.resources {
        let args = resource_args(&run.root, target, profile, fixture, rc)?;
        run.required(
            profile["resource_compiler"]
                .as_str()
                .ok_or("resource compiler")?,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        let res = target.join(format!("{}.res", fixture.name));
        let header = run.root.join(DIRECTORY).join("editor.h");
        resources = json!({"script":format!("{DIRECTORY}/{rc}"),"script_sha256":file_hash(&run.root.join(DIRECTORY).join(rc))?,
            "header_sha256":file_hash(&header)?,"res":relative(&run.root,&res)?,"res_sha256":file_hash(&res)?});
    }
    headers.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    headers.dedup_by(|a, b| a["path"] == b["path"]);
    let link = link_args(target, profile, fixture)?;
    run.required(
        &super::build::executable(profile, "link.exe")?,
        &link.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let path = target.join(format!("{}.exe", fixture.name));
    let map = target.join(format!("{}.map", fixture.name));
    let inspected = artifact::inspect(&path, "windows-fixture")?;
    if inspected["inspection"]["subsystem"] != 2 {
        return Err(format!(
            "{} is not a Windows GUI subsystem executable",
            fixture.name
        ));
    }
    let metadata = super::pe::inspect(&path)?;
    let modules = imports(&metadata)?;
    if metadata["preferred_base"] != 0x140000000u64 {
        return Err(format!("{} preferred base differs", fixture.name));
    }
    let commands: Vec<_> = run.commands()[start..]
        .iter()
        .map(|c| c["id"].clone())
        .collect();
    Ok(
        json!({"id":fixture.id,"name":fixture.name,"source":format!("{DIRECTORY}/{}",fixture.source),
        "source_sha256":file_hash(&run.root.join(DIRECTORY).join(fixture.source))?,"defines":fixture.defines,
        "resources":resources,"path":relative(&run.root,&path)?,"artifact":inspected,"pe":metadata,"imports":modules,
        "initfs_destination":format!("seed/{}.exe",fixture.name),"install_path":format!("/system/bin/{}.exe",fixture.name),
        "headers":headers,"commands":commands,"link_map":{"path":relative(&run.root,&map)?,"sha256":file_hash(&map)?},
        "runtime":"independent Windows SDK C; no CRT; /SUBSYSTEM:WINDOWS"}),
    )
}

/// One build of every fixture into `target`.
pub fn build(run: &mut Run, target: &Path) -> Result<Vec<Value>, String> {
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    let profile = toolchain(&run.root)?;
    FIXTURES
        .iter()
        .map(|fixture| build_one(run, target, &profile, fixture))
        .collect()
}

/// Two independent builds whose executables must be byte-identical. Returns
/// the first pass and the reproducibility record.
pub fn double_build(run: &mut Run, base: &Path) -> Result<(Vec<Value>, Value), String> {
    let first = build(run, &base.join("pass-0"))?;
    let second = build(run, &base.join("pass-1"))?;
    let mut record = Vec::new();
    for (a, b) in first.iter().zip(&second) {
        if a["name"] != b["name"] || a["artifact"]["sha256"] != b["artifact"]["sha256"] {
            return Err(format!(
                "independent Windows GUI fixture rebuild differs: {}",
                a["name"]
            ));
        }
        record.push(json!({"name":a["name"],"sha256":a["artifact"]["sha256"],"paths":[a["path"],b["path"]]}));
    }
    Ok((first, json!(record)))
}

/// The import inventory of built fixtures: per fixture, and the union by module.
pub fn inventory(artifacts: &[Value]) -> Result<Value, String> {
    let mut union: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut fixtures = Vec::new();
    for artifact in artifacts {
        let modules = artifact["imports"]
            .as_object()
            .ok_or("fixture import map")?;
        for (module, symbols) in modules {
            let entry = union.entry(module.clone()).or_default();
            for symbol in symbols.as_array().ok_or("fixture symbols")? {
                entry.push(symbol.as_str().ok_or("fixture symbol")?.to_owned());
            }
        }
        fixtures.push(json!({"id":artifact["id"],"name":artifact["name"],"source":artifact["source"],
            "sha256":artifact["artifact"]["sha256"],"subsystem":artifact["artifact"]["inspection"]["subsystem"],
            "imports":artifact["imports"]}));
    }
    for symbols in union.values_mut() {
        symbols.sort();
        symbols.dedup();
    }
    let counts: BTreeMap<_, _> = union.iter().map(|(m, s)| (m.clone(), s.len())).collect();
    Ok(
        json!({"schema_version":1,"record_kind":"windows_gui_import_inventory",
        "scope":"Named imports of the P13 Win32 GUI fixtures (pe::inspect_bytes over the reproducible build). A Door compatibility DLL must export every symbol of its module; a missing export fails the load (no success stubs).",
        "toolchain":{"msvc":"14.44.35207","sdk":"10.0.19041.0","import_libraries":GUI_LIBRARIES},
        "fixtures":fixtures,"union":union,"counts":counts}),
    )
}

/// The published inventory must describe exactly the current fixture bytes.
pub fn check_published(root: &Path, artifacts: &[Value]) -> Result<Value, String> {
    let computed = inventory(artifacts)?;
    let published = image::read_json(&paths::existing(root, INVENTORY)?)?;
    if published != computed {
        return Err(format!(
            "{INVENTORY} is stale; regenerate it with cargo xtask build --profile windows-gui-reference"
        ));
    }
    let reference = image::read_json(&paths::existing(root, REFERENCE)?)?;
    for artifact in artifacts {
        let name = artifact["name"].as_str().ok_or("fixture name")?;
        if reference["fixtures"][name]["sha256"] != artifact["artifact"]["sha256"] {
            return Err(format!(
                "{name} bytes differ from the Windows reference record ({REFERENCE}); rerun cargo xtask build --profile windows-gui-reference"
            ));
        }
    }
    Ok(
        json!({"inventory":INVENTORY,"inventory_sha256":file_hash(&root.join(INVENTORY))?,
        "reference":REFERENCE,"reference_sha256":file_hash(&root.join(REFERENCE))?}),
    )
}

fn identity(run: &mut Run) -> Result<(Value, Value), String> {
    let observed = run.required("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", "$referenceOS = Get-CimInstance Win32_OperatingSystem; [pscustomobject]@{caption=$referenceOS.Caption;version=$referenceOS.Version;build=$referenceOS.BuildNumber;architecture=$referenceOS.OSArchitecture} | ConvertTo-Json -Compress"])?;
    let command = run.commands().last().ok_or("Windows identity command")?["id"].clone();
    let identity: Value =
        serde_json::from_str(observed.stdout.trim()).map_err(|e| e.to_string())?;
    Ok((identity, command))
}

/// Read-only snapshot of the per-user common-dialog and recent-document
/// registry state, to prove the reference run wrote no MRU entries.
fn registry_snapshot(run: &mut Run) -> Result<Value, String> {
    let mut keys = Vec::new();
    for key in [
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\RecentDocs",
    ] {
        let observed = run.optional("reg.exe", &["query", key, "/s"])?;
        use sha2::{Digest, Sha256};
        let digest: String = Sha256::digest(observed.stdout.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        keys.push(json!({"key":key,"exit":observed.exit_code,"sha256":digest,"bytes":observed.stdout.len()}));
    }
    Ok(json!(keys))
}

fn probe_metrics(results: &str) -> Value {
    let mut probes = BTreeMap::new();
    let mut traces = BTreeMap::new();
    let mut raster = Value::Null;
    for line in results.lines() {
        if let Some(rest) = line.strip_prefix("WINDOWS_GUI_PROBE raster ") {
            raster = json!(rest);
        } else if let Some(rest) = line.strip_prefix("WINDOWS_GUI_PROBE name=") {
            let mut words = rest.split(' ');
            let name = words.next().unwrap_or("").to_owned();
            let fields: BTreeMap<_, _> = words
                .filter_map(|w| w.split_once('='))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect();
            probes.insert(name, fields);
        } else if let Some(rest) = line.strip_prefix("WINDOWS_GUI_TRACE phase=")
            && let Some((phase, entries)) = rest.split_once(" entries=")
        {
            traces.insert(phase.to_owned(), entries.to_owned());
        }
    }
    json!({"probes":probes,"traces":traces,"raster":raster})
}

/// The Windows reference run on the owner's PC: read-only (no input injection
/// into other applications, no setting changes). Each fixture runs in a fresh
/// working directory under `out/`; every selftest runs twice (fresh
/// directories) and must produce identical results.
pub fn reference(run: &mut Run, artifacts: &[Value], base: &Path) -> Result<Value, String> {
    // The Explorer-style dialogs update their ComDlg32 MRU lists even with
    // OFN_DONTADDTORECENT: snapshot the key's values and, even when the run
    // fails, undo exactly the run's changes (diff-based, never deleting the
    // key). The snapshot is kept in the working directory and the run.
    let before_registry = registry_snapshot(run)?;
    let snapshot = base.join("comdlg32-before.reg");
    let exported = export_key(run, COMDLG32_KEY, &snapshot)?;
    let before = match &exported {
        Some(_) => RegExport::parse(&read_reg(&snapshot)?)?,
        None => RegExport::default(),
    };
    if exported.is_some() {
        fs::copy(&snapshot, run.directory.join("comdlg32-before.reg"))
            .map_err(|e| format!("keep ComDlg32 snapshot: {e}"))?;
    }
    let result = reference_runs(run, artifacts, base, &before_registry);
    let restored = restore_values(run, base, &before);
    let check = base.join("comdlg32-restored.reg");
    let after = restored.and_then(|diff| {
        let hash = export_key(run, COMDLG32_KEY, &check)?;
        let state = match hash {
            Some(_) => RegExport::parse(&read_reg(&check)?)?,
            None => RegExport::default(),
        };
        Ok((diff, hash, state))
    });
    let mut record = result?;
    let (diff, restored_hash, state) = after?;
    let restored_registry = registry_snapshot(run)?;
    // A key that did not exist before is left empty, never deleted.
    let subkeys_restored = if exported.is_some() {
        state.subkeys == before.subkeys
    } else {
        state
            .subkeys
            .iter()
            .all(|key| key.eq_ignore_ascii_case(&comdlg32_root()))
    };
    if state.values != before.values
        || !subkeys_restored
        || restored_registry[1] != before_registry[1]
    {
        return Err(format!(
            "ComDlg32 MRU values were not restored exactly (export {exported:?} -> {restored_hash:?})"
        ));
    }
    record["registry"]["comdlg32_restore"] = json!({"key":COMDLG32_KEY,
        "method":"reg export snapshot before the run; afterwards only values absent from the snapshot are deleted (and subkeys the run created), and changed or missing values are re-imported byte-for-byte from the snapshot's own data; the key itself is never deleted",
        "snapshot":"comdlg32-before.reg","export_before_sha256":exported,"export_after_restore_sha256":restored_hash,
        "values_restored_exactly":true,"diff":diff,"query_after_restore":restored_registry});
    record["registry"]["note"] = json!(
        "OFN_DONTADDTORECENT keeps RecentDocs unchanged; the Explorer-style dialog still updates its ComDlg32 MRU lists during the run; the run's changes to the key are undone value by value afterwards"
    );
    Ok(record)
}
const COMDLG32_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32";
/// COMDLG32_KEY as `reg export` spells it.
fn comdlg32_root() -> String {
    format!(
        "HKEY_CURRENT_USER{}",
        COMDLG32_KEY.strip_prefix("HKCU").unwrap_or(COMDLG32_KEY)
    )
}
/// Verbatim path prefix that reg.exe does not accept.
const VERBATIM: &str = "\\\\?\\";
/// `reg export` of `key` to `file`; the export's SHA-256, or None when the
/// key does not exist.
fn export_key(run: &mut Run, key: &str, file: &Path) -> Result<Option<String>, String> {
    let path = file
        .to_str()
        .ok_or("registry export path")?
        .trim_start_matches(VERBATIM);
    let observed = run.optional("reg.exe", &["export", key, path, "/y"])?;
    if observed.exit_code != Some(0) {
        let query = run.optional("reg.exe", &["query", key])?;
        if query.exit_code == Some(0) {
            return Err(format!("reg export of {key} failed"));
        }
        return Ok(None);
    }
    file_hash(file).map(Some)
}
/// A `reg export` file (UTF-16LE with BOM, or UTF-8) as text.
fn read_reg(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        return String::from_utf16(&units).map_err(|e| e.to_string());
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}
/// The values of a `reg export`: `(subkey, name)` -> (the export's own name
/// token, data text with continuation lines joined), so re-importing a value
/// writes exactly its exported bytes.
#[derive(Debug, Default, PartialEq)]
struct RegExport {
    subkeys: std::collections::BTreeSet<String>,
    values: BTreeMap<(String, String), (String, String)>,
}
impl RegExport {
    fn parse(text: &str) -> Result<Self, String> {
        let mut export = Self::default();
        let mut subkey: Option<String> = None;
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            let line = line.trim_end_matches('\r');
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                export.subkeys.insert(name.to_owned());
                subkey = Some(name.to_owned());
                continue;
            }
            if line.is_empty() || line.starts_with("Windows Registry Editor") {
                continue;
            }
            let key = subkey.clone().ok_or("registry value outside a key")?;
            let (token, name, rest) = if let Some(rest) = line.strip_prefix('@') {
                ("@".to_owned(), String::new(), rest)
            } else if line.starts_with('"') {
                let mut name = String::new();
                let mut escaped = false;
                let mut end = None;
                for (index, ch) in line.char_indices().skip(1) {
                    match (escaped, ch) {
                        (true, _) => {
                            name.push(ch);
                            escaped = false;
                        }
                        (false, '\\') => escaped = true,
                        (false, '"') => {
                            end = Some(index);
                            break;
                        }
                        (false, _) => name.push(ch),
                    }
                }
                let end = end.ok_or("unterminated registry value name")?;
                (line[..=end].to_owned(), name, &line[end + 1..])
            } else {
                return Err(format!("unrecognized registry export line {line:?}"));
            };
            let mut data = rest
                .strip_prefix('=')
                .ok_or("registry value without data")?
                .to_owned();
            // Hex data continues on indented lines after a trailing backslash.
            while data.starts_with("hex") && data.ends_with('\\') {
                data.pop();
                let next = lines.next().ok_or("unterminated registry hex data")?;
                data.push_str(next.trim_end_matches('\r').trim_start());
            }
            export.values.insert((key, name), (token, data));
        }
        Ok(export)
    }
}
/// What returns the current key to `before`: values to delete, subkeys the
/// run created (deepest first; never the snapshot's own keys), and a .reg
/// text re-importing every changed or missing value from the snapshot.
#[derive(Debug, PartialEq)]
struct RegRestore {
    delete_values: Vec<(String, String)>,
    delete_subkeys: Vec<String>,
    import: Option<String>,
}
fn plan_restore(before: &RegExport, current: &RegExport, root: &str) -> RegRestore {
    let delete_values = current
        .values
        .keys()
        .filter(|key| !before.values.contains_key(*key))
        .cloned()
        .collect();
    let mut delete_subkeys: Vec<String> = current
        .subkeys
        .iter()
        .filter(|key| !before.subkeys.contains(*key) && !key.eq_ignore_ascii_case(root))
        .cloned()
        .collect();
    delete_subkeys.sort_by_key(|key| std::cmp::Reverse(key.matches('\\').count()));
    let mut import = String::from("Windows Registry Editor Version 5.00\r\n");
    let mut section: Option<&str> = None;
    let mut any = false;
    for ((key, name), (token, data)) in &before.values {
        if current.values.get(&(key.clone(), name.clone())) == Some(&(token.clone(), data.clone()))
        {
            continue;
        }
        if section != Some(key.as_str()) {
            import.push_str(&format!("\r\n[{key}]\r\n"));
            section = Some(key.as_str());
        }
        import.push_str(&format!("{token}={data}\r\n"));
        any = true;
    }
    RegRestore {
        delete_values,
        delete_subkeys,
        import: any.then_some(import),
    }
}
/// Undoes the run's changes to the ComDlg32 key relative to `before`.
fn restore_values(run: &mut Run, base: &Path, before: &RegExport) -> Result<Value, String> {
    let current_file = base.join("comdlg32-after-run.reg");
    let current = match export_key(run, COMDLG32_KEY, &current_file)? {
        Some(_) => RegExport::parse(&read_reg(&current_file)?)?,
        None => RegExport::default(),
    };
    let plan = plan_restore(before, &current, &comdlg32_root());
    for (key, name) in &plan.delete_values {
        if plan.delete_subkeys.iter().any(|k| k == key) {
            continue;
        }
        let mut args = vec!["delete", key.as_str()];
        if name.is_empty() {
            args.push("/ve");
        } else {
            args.extend(["/v", name.as_str()]);
        }
        args.push("/f");
        run.required("reg.exe", &args)?;
    }
    for key in &plan.delete_subkeys {
        run.required("reg.exe", &["delete", key, "/f"])?;
    }
    if let Some(text) = &plan.import {
        let file = base.join("comdlg32-restore.reg");
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        fs::write(&file, bytes).map_err(|e| e.to_string())?;
        let path = file
            .to_str()
            .ok_or("registry restore path")?
            .trim_start_matches(VERBATIM);
        run.required("reg.exe", &["import", path])?;
    }
    let names = |list: &[(String, String)]| -> Vec<String> {
        list.iter().map(|(k, n)| format!("{k}\\{n}")).collect()
    };
    let restored: Vec<String> = before
        .values
        .iter()
        .filter(|(key, value)| current.values.get(*key) != Some(*value))
        .map(|((k, n), _)| format!("{k}\\{n}"))
        .collect();
    Ok(
        json!({"deleted_values":names(&plan.delete_values),"deleted_subkeys":plan.delete_subkeys,
        "restored_values":restored,"key_deleted":false}),
    )
}
fn reference_runs(
    run: &mut Run,
    artifacts: &[Value],
    base: &Path,
    before_registry: &Value,
) -> Result<Value, String> {
    let (identity, identity_command) = identity(run)?;
    let mut hashes = BTreeMap::new();
    for artifact in artifacts {
        let path = paths::existing(&run.root, artifact["path"].as_str().ok_or("fixture path")?)?;
        if file_hash(&path)? != artifact["artifact"]["sha256"] {
            return Err("Windows GUI reference binary changed before launch".into());
        }
        hashes.insert(
            artifact["name"].as_str().ok_or("fixture name")?.to_owned(),
            path,
        );
    }
    let mut launches = Vec::new();
    let mut directories: BTreeMap<String, std::path::PathBuf> = BTreeMap::new();
    for pass in 0..2 {
        for launch in &LAUNCHES {
            // The second pass repeats each self-test once, in fresh directories.
            if pass == 1
                && launch.argument != "--selftest"
                && !(launch.fixture == "wingui-selftest" && launch.argument.is_empty())
            {
                continue;
            }
            let key = format!("{}-{pass}", launch.directory);
            let directory = match directories.get(&key) {
                Some(d) => d.clone(),
                None => {
                    let d = base.join(&key);
                    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
                    directories.insert(key.clone(), d.clone());
                    d
                }
            };
            let exe = &hashes[launch.fixture];
            let args: Vec<&str> = if launch.argument.is_empty() {
                vec![]
            } else {
                vec![launch.argument]
            };
            let observed = run.watched_in(
                &directory,
                // The ordinary drive-letter path: the shell keys per-application
                // dialog state by the module path and mishandles verbatim paths.
                {
                    let path = exe.to_str().ok_or("fixture executable")?;
                    path.strip_prefix(r"\\?\").unwrap_or(path)
                },
                &args,
                Duration::from_secs(launch.timeout_seconds),
                launch.exit,
            )?;
            let command = run.commands().last().ok_or("reference command")?["id"].clone();
            if !observed.success || !observed.stderr.is_empty() {
                return Err(format!(
                    "Windows GUI reference {} {} failed (exit {:?}): {:?}",
                    launch.fixture, launch.argument, observed.exit_code, observed.stdout
                ));
            }
            launches.push(json!({"fixture":launch.fixture,"argument":launch.argument,"exit":launch.exit,
                "directory":key,"results_file":launch.results,"stdout":observed.stdout,"command":command,"pass":pass}));
        }
    }
    // Results files: complete line-based records, equal to the concatenated
    // standard output of the runs that share them, and identical across passes.
    let mut results = BTreeMap::new();
    for launch in &LAUNCHES {
        let first = base
            .join(format!("{}-0", launch.directory))
            .join(launch.results);
        let text = fs::read_to_string(&first).map_err(|e| format!("{}: {e}", first.display()))?;
        let stdout: String = launches
            .iter()
            .filter(|l| {
                l["pass"] == 0
                    && l["directory"] == format!("{}-0", launch.directory)
                    && l["results_file"] == launch.results
            })
            .map(|l| l["stdout"].as_str().unwrap_or(""))
            .collect();
        if text != stdout || !text.ends_with("\r\n") {
            return Err(format!(
                "{} results differ from standard output",
                launch.results
            ));
        }
        let second = base
            .join(format!("{}-1", launch.directory))
            .join(launch.results);
        if second.is_file() {
            let repeated = fs::read_to_string(&second).map_err(|e| e.to_string())?;
            if !text.starts_with(&repeated) {
                return Err(format!(
                    "{} self-test results are not deterministic",
                    launch.results
                ));
            }
        }
        results.insert(
            format!("{}/{}", launch.directory, launch.results),
            json!({"text":text,"sha256":file_hash(&first)?,"bytes":text.len()}),
        );
    }
    let mut saved = BTreeMap::new();
    for (directory, name) in SAVED {
        let path = base.join(format!("{directory}-0")).join(name);
        saved.insert(
            format!("{directory}/{name}"),
            json!({"sha256":file_hash(&path)?,"bytes":fs::metadata(&path).map_err(|e| e.to_string())?.len()}),
        );
    }
    let raster = base.join("f1-probe-0/wingui-raster.bin");
    let probe = results["f1-probe/wingui-selftest-probe.results"]["text"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    // Recorded, not hidden: the Explorer-style common dialog updates its own
    // ComDlg32 MRU lists (FirstFolder, LastVisitedPidlMRU, OpenSavePidlMRU,
    // CIDSizeMRU) even with OFN_DONTADDTORECENT, which only suppresses the
    // Recent Items shortcut. Nothing is restored: that would itself write.
    let after_registry = registry_snapshot(run)?;
    if before_registry[1] != after_registry[1] {
        return Err(
            "Windows reference run added recent documents despite OFN_DONTADDTORECENT".into(),
        );
    }
    for (name, path) in &hashes {
        let artifact = artifacts
            .iter()
            .find(|a| a["name"] == name.as_str())
            .ok_or("fixture")?;
        if file_hash(path)? != artifact["artifact"]["sha256"] {
            return Err("Windows GUI reference binary bytes changed".into());
        }
    }
    let fixtures: BTreeMap<_, _> = artifacts
        .iter()
        .map(|a| {
            (a["name"].as_str().unwrap_or("").to_owned(),
                json!({"id":a["id"],"sha256":a["artifact"]["sha256"],"source_sha256":a["source_sha256"]}))
        })
        .collect();
    Ok(
        json!({"schema_version":1,"record_kind":"windows_gui_reference","outcome":"passed",
        "identity":identity,"identity_command":identity_command,"fixtures":fixtures,
        "launches":launches.iter().map(|l| json!({"fixture":l["fixture"],"argument":l["argument"],"exit":l["exit"],"directory":l["directory"],"results_file":l["results_file"],"command":l["command"],"pass":l["pass"]})).collect::<Vec<_>>(),
        "results":results,"saved":saved,
        "metrics":{"f1_probe":probe_metrics(&probe),"raster":{"sha256":file_hash(&raster)?,"bytes":fs::metadata(&raster).map_err(|e| e.to_string())?.len()}},
        "registry":{"before":before_registry,"after":after_registry,"recent_documents_unchanged":true,"common_dialog_mru_changed":before_registry[0] != after_registry[0],"note":"OFN_DONTADDTORECENT keeps RecentDocs unchanged; the Explorer-style dialog still updates its ComDlg32 MRU lists; not restored (restoring would write the registry)"},
        "policy":"Owner-approved read-only reference: small fixture windows open briefly on the owner's desktop; every input is a message to the fixture's own windows; no input injection, no setting changes; dialogs pass OFN_DONTADDTORECENT",
        "watcher_timeout_seconds":60,"binary_modified":false}),
    )
}

/// `cargo xtask build --profile windows-gui-reference`: double build, import
/// inventory, Windows reference run, and the two published records.
pub fn standalone(run: &mut Run) -> Result<(), String> {
    let target = paths::fresh_directory(&run.root, &format!("out/builds/{}/windows-gui", run.id))?;
    let (artifacts, reproducibility) = double_build(run, &target)?;
    let inventory = inventory(&artifacts)?;
    write_json(&run.root.join(INVENTORY), &inventory)?;
    run.check("p13-windows-gui-inventory", json!({"path":INVENTORY,"sha256":file_hash(&run.root.join(INVENTORY))?,"counts":inventory["counts"]}));
    let working =
        paths::fresh_directory(&run.root, &format!("out/windows-gui-reference/{}", run.id))?;
    let reference = reference(run, &artifacts, &working)?;
    write_json(&run.root.join(REFERENCE), &reference)?;
    let record = json!({"artifacts":artifacts,"reproducibility":reproducibility,"inventory":INVENTORY,
        "reference":REFERENCE,"reference_sha256":file_hash(&run.root.join(REFERENCE))?,
        "working_directory":relative(&run.root,&working)?,
        "scope":"Windows reference only; Door guest qualification remains required (cargo xtask test compat --suite win32-gui-f1|f2|f3)"});
    write_json(&run.directory.join("windows-gui-reference.json"), &record)?;
    run.check("p13-windows-gui-reference", record);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const ROOT: &str =
        r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32";
    fn export(body: &str) -> RegExport {
        RegExport::parse(&format!(
            "Windows Registry Editor Version 5.00\r\n\r\n{body}"
        ))
        .unwrap()
    }
    #[test]
    fn registry_restore_deletes_only_new_values_and_reimports_changed_bytes() {
        let before = export(&format!(
            "[{ROOT}]\r\n\"Kept\"=dword:00000001\r\n\r\n[{ROOT}\\OpenSavePidlMRU\\txt]\r\n\
             \"MRUListEx\"=hex:01,00,00,00,00,00,00,00,\\\r\n  ff,ff,ff,ff\r\n\"0\"=hex:14,00\r\n\
             @=\"default \\\"quoted\\\" C:\\\\\"\r\n"
        ));
        let mru = (
            format!(r"{ROOT}\OpenSavePidlMRU\txt"),
            "MRUListEx".to_owned(),
        );
        assert_eq!(
            before.values[&mru].1,
            "hex:01,00,00,00,00,00,00,00,ff,ff,ff,ff"
        );
        let default = (format!(r"{ROOT}\OpenSavePidlMRU\txt"), String::new());
        assert_eq!(before.values[&default].0, "@");
        // The run added entries 1 and 2, rewrote MRUListEx, and created a
        // new list for another extension.
        let after = export(&format!(
            "[{ROOT}]\r\n\"Kept\"=dword:00000001\r\n\r\n[{ROOT}\\OpenSavePidlMRU\\txt]\r\n\
             \"MRUListEx\"=hex:02,00,00,00,01,00,00,00,00,00,00,00,ff,ff,ff,ff\r\n\"0\"=hex:14,00\r\n\
             \"1\"=hex:16,00\r\n\"2\"=hex:18,00\r\n@=\"default \\\"quoted\\\" C:\\\\\"\r\n\r\n\
             [{ROOT}\\OpenSavePidlMRU\\doors]\r\n\"0\"=hex:20,00\r\n"
        ));
        let plan = plan_restore(&before, &after, ROOT);
        let txt = format!(r"{ROOT}\OpenSavePidlMRU\txt");
        let doors = format!(r"{ROOT}\OpenSavePidlMRU\doors");
        assert_eq!(
            plan.delete_values,
            [
                (doors.clone(), "0".to_owned()),
                (txt.clone(), "1".to_owned()),
                (txt.clone(), "2".to_owned())
            ]
        );
        assert_eq!(plan.delete_subkeys, [doors]);
        assert_eq!(
            plan.import.as_deref(),
            Some(
                format!(
                    "Windows Registry Editor Version 5.00\r\n\r\n[{txt}]\r\n\"MRUListEx\"=hex:01,00,00,00,00,00,00,00,ff,ff,ff,ff\r\n"
                )
                .as_str()
            )
        );
        // Re-importing the plan's text yields the snapshot's values.
        let reimported = export(
            plan.import
                .as_deref()
                .unwrap()
                .trim_start_matches("Windows Registry Editor Version 5.00\r\n"),
        );
        assert_eq!(reimported.values[&mru], before.values[&mru]);
        // Nothing to undo: no deletions and no import; the key itself is
        // never among the deletions, even when the snapshot had none.
        let same = plan_restore(&before, &before, ROOT);
        assert!(same.delete_values.is_empty() && same.delete_subkeys.is_empty());
        assert!(same.import.is_none());
        let fresh = plan_restore(&RegExport::default(), &after, ROOT);
        assert!(!fresh.delete_subkeys.iter().any(|k| k == ROOT));
        assert!(RegExport::parse(&format!("[{ROOT}]\r\nbogus\r\n")).is_err());
    }
    #[test]
    fn launches_and_saved_documents_name_declared_fixtures() {
        for launch in &LAUNCHES {
            assert!(FIXTURES.iter().any(|f| f.name == launch.fixture));
            assert!(
                launch.timeout_seconds > 10,
                "IsHungAppWindow needs more than 10 s"
            );
        }
        for (directory, _) in SAVED {
            assert!(LAUNCHES.iter().any(|l| l.directory == directory));
        }
    }
    #[test]
    fn imports_reject_unknown_modules_and_ordinals() {
        let ok = json!({"imports":[{"module":"KERNEL32.dll","symbols":[{"name":"ExitProcess"}]},{"module":"USER32.dll","symbols":[{"name":"GetMessageW"},{"name":"CreateWindowExW"}]}]});
        let modules = imports(&ok).unwrap();
        assert_eq!(modules["user32.dll"], ["CreateWindowExW", "GetMessageW"]);
        let foreign = json!({"imports":[{"module":"KERNEL32.dll","symbols":[]},{"module":"USER32.dll","symbols":[]},{"module":"shell32.dll","symbols":[]}]});
        assert!(imports(&foreign).is_err());
        let ordinal = json!({"imports":[{"module":"KERNEL32.dll","symbols":[]},{"module":"USER32.dll","symbols":[{"ordinal":5}]}]});
        assert!(imports(&ordinal).is_err());
        assert!(imports(&json!({"imports":[{"module":"KERNEL32.dll","symbols":[]}]})).is_err());
    }
    #[test]
    fn gui_links_select_the_windows_subsystem_and_every_gui_library() {
        let profile = json!({"sdk_root":"C:/SDK","sdk_version":"10.0.19041.0"});
        let args = link_args(Path::new("C:/t"), &profile, &FIXTURES[2]).unwrap();
        assert!(args.contains(&"/SUBSYSTEM:WINDOWS,6.02".to_owned()));
        assert!(args.iter().any(|a| a.ends_with("wingui-editor-rc.res")));
        for library in GUI_LIBRARIES {
            assert!(args.iter().any(|a| a.ends_with(library)));
        }
    }
    #[test]
    fn published_inventory_matches_the_declared_fixture_set() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let inventory = image::read_json(&root.join(INVENTORY)).unwrap();
        let names: Vec<_> = inventory["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].clone())
            .collect();
        assert_eq!(
            names,
            FIXTURES.iter().map(|f| json!(f.name)).collect::<Vec<_>>()
        );
        for module in inventory["union"].as_object().unwrap().keys() {
            assert!(ALLOWED_MODULES.contains(&module.as_str()));
        }
    }
}
