use crate::{
    evidence::{Observation, Run, file_hash, relative, write_json},
    image, paths,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

/// P18: the WHPX desktop with two declared CPUs (the kernel's device
/// profile runs on the boot CPU, the other stays parked until the SMP
/// scheduler): programs see a 2-CPU machine (Electron's single-CPU start-up
/// path checks a state its own constructor has already changed).
pub const DESKTOP_SMP2: &str = "qemu-desktop-whpx-smp2-x64";
fn desktop_id(profile: &Value) -> bool {
    profile["id"] == "qemu-desktop-whpx-x64" || profile["id"] == DESKTOP_SMP2
}
pub fn validate_profile(profile: &Value) -> Result<(), String> {
    let data_serial = match profile["id"].as_str() {
        Some("qemu-storage-x64") => Some("DOOR-P06-DATA"),
        Some("qemu-persistence-x64") => Some("DOOR-P07-DATA"),
        Some("qemu-services-x64" | "qemu-desktop-whpx-x64" | DESKTOP_SMP2) => Some("DOOR-P08-DATA"),
        Some("qemu-windows-x64") => Some("DOOR-P09-DATA"),
        _ => None,
    };
    let desktop_cpus = if profile["id"] == DESKTOP_SMP2 { 2 } else { 1 };
    if !profile["data_disk"].is_null()
        && (data_serial.is_none()
            || profile["vcpus"] != desktop_cpus
            || profile["memory_mib"] != 2048
            || profile["data_disk"]
                != json!({"format":"raw","bytes":134217728,"transport":"virtio-blk-modern","writable_targets_must_be_workspace_local":true,"serial":data_serial}))
    {
        return Err("unsupported writable storage profile".into());
    }
    if profile["schema_version"] != 1
        || profile["record_kind"] != "hardware_profile"
        || profile["architecture"] != "amd64"
        || profile["firmware_kind"] != "uefi"
        || !(profile["accelerator"] == "tcg" && !desktop_id(profile)
            || profile["accelerator"] == "whpx"
                && desktop_id(profile)
                && profile["vcpus"] == desktop_cpus
                && profile["memory_mib"] == 2048)
        || desktop_id(profile) && profile["display"]["device"] != "ramfb"
        || !matches!(profile["vcpus"].as_u64(), Some(1 | 2 | 4))
        || !matches!(profile["memory_mib"].as_u64(), Some(256 | 2048))
        || profile["network"]["enabled"] != false
        || profile["physical_disk_passthrough"] != false
    {
        return Err("unsupported diagnostic hardware profile".into());
    }
    if profile["vcpus"] != 1
        && profile["id"] != DESKTOP_SMP2
        && (profile["memory_mib"] != 2048
            || profile["id"]
                != format!(
                    "qemu-smp{}-x64",
                    profile["vcpus"].as_u64().ok_or("CPU count")?
                ))
    {
        return Err("SMP requires its distinct pinned 2 GiB profile identity".into());
    }
    let machine = profile["machine_version"]
        .as_str()
        .ok_or("versioned q35 machine not acquired")?;
    let suffix = machine
        .strip_prefix("pc-q35-")
        .ok_or("versioned q35 machine required")?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err("invalid machine version".into());
    }
    for key in ["cpu_model", "qemu_version"] {
        let value = profile[key]
            .as_str()
            .ok_or_else(|| format!("{key} not acquired"))?;
        if value.is_empty()
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(format!("invalid {key}"));
        }
    }
    for (object, path_key, hash_key) in [
        ("qemu", "path", "sha256"),
        ("firmware", "code_path", "code_sha256"),
        ("firmware", "vars_template", "vars_template_sha256"),
    ] {
        if profile[object][path_key]
            .as_str()
            .is_none_or(|v| v.is_empty())
        {
            return Err(format!("{object}.{path_key} not acquired"));
        }
        if !profile[object][hash_key].as_str().is_some_and(|v| {
            v.len() == 64
                && v.bytes()
                    .all(|b| b.is_ascii_digit() || b"abcdef".contains(&b))
        }) {
            return Err(format!("invalid {object}.{hash_key}"));
        }
    }
    Ok(())
}
fn acquired_file(
    profile: &Value,
    object: &str,
    path_key: &str,
    hash_key: &str,
) -> Result<PathBuf, String> {
    let path = Path::new(
        profile[object][path_key]
            .as_str()
            .ok_or("missing acquisition path")?,
    );
    if !path.is_absolute() || !path.is_file() {
        return Err("acquired tool/firmware path must be an existing absolute file".into());
    }
    if file_hash(path)?
        != profile[object][hash_key]
            .as_str()
            .ok_or("missing acquisition digest")?
    {
        return Err("acquired tool/firmware hash mismatch".into());
    }
    Ok(path.to_path_buf())
}
fn escaped(path: &Path) -> Result<String, String> {
    Ok(path
        .to_str()
        .ok_or("non-UTF8 emulator path")?
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/")
        .replace(',', ",,"))
}
pub fn arguments(
    profile: &Value,
    code: &Path,
    vars: &Path,
    esp: &Path,
    serial: &Path,
    token: &str,
) -> Result<Vec<String>, String> {
    validate_profile(profile)?;
    if token.is_empty()
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("invalid run token".into());
    }
    Ok(vec![
        "-machine".into(),
        profile["machine_version"]
            .as_str()
            .ok_or("machine missing")?
            .into(),
        "-accel".into(),
        profile["accelerator"]
            .as_str()
            .ok_or("accelerator missing")?
            .into(),
        "-cpu".into(),
        profile["cpu_model"].as_str().ok_or("CPU missing")?.into(),
        "-smp".into(),
        profile["vcpus"].as_u64().ok_or("CPU count")?.to_string(),
        "-m".into(),
        profile["memory_mib"]
            .as_u64()
            .ok_or("memory size")?
            .to_string(),
        "-nodefaults".into(),
        "-device".into(),
        "VGA".into(),
        "-display".into(),
        "none".into(),
        "-monitor".into(),
        "none".into(),
        "-nic".into(),
        "none".into(),
        "-no-reboot".into(),
        "-serial".into(),
        format!("file:{}", serial.to_str().ok_or("serial path")?),
        "-drive".into(),
        format!(
            "if=pflash,format=raw,unit=0,readonly=on,file={}",
            escaped(code)?
        ),
        "-drive".into(),
        format!("if=pflash,format=raw,unit=1,file={}", escaped(vars)?),
        "-drive".into(),
        format!(
            "if=none,id=door-esp,format=raw,readonly=on,file=fat:{}",
            escaped(esp)?
        ),
        "-device".into(),
        "virtio-blk-pci,drive=door-esp".into(),
        "-device".into(),
        "isa-debug-exit,iobase=0xf4,iosize=0x04".into(),
        "-fw_cfg".into(),
        format!("name=opt/door/test-token,string={token}"),
    ])
}
/// P15: a modern VirtIO NIC on QEMU's user-mode network (DHCP lease
/// 10.0.2.15, gateway 10.0.2.2, DNS 10.0.2.3; nothing reaches the guest
/// from outside unless a test forwards a port).
pub fn network_arguments() -> [String; 4] {
    let mut netdev = String::from("user,id=door-net");
    for (host, guest) in FORWARDS.lock().map(|f| f.clone()).unwrap_or_default() {
        netdev.push_str(&format!(",hostfwd=tcp:127.0.0.1:{host}-:{guest}"));
    }
    [
        "-netdev".into(),
        netdev,
        "-device".into(),
        "virtio-net-pci,netdev=door-net,disable-legacy=on,mac=52:54:00:d0:05:01".into(),
    ]
}
/// TCP ports forwarded from the host's loopback into the guest for the
/// guests launched while a [`Forwarding`] is alive (CONNECT-A server tests).
static FORWARDS: std::sync::Mutex<Vec<(u16, u16)>> = std::sync::Mutex::new(Vec::new());
/// Forwards host `127.0.0.1:host` to guest port `guest` until dropped.
pub struct Forwarding;
impl Drop for Forwarding {
    fn drop(&mut self) {
        if let Ok(mut forwards) = FORWARDS.lock() {
            forwards.clear();
        }
    }
}
pub fn forward_tcp(pairs: &[(u16, u16)]) -> Forwarding {
    if let Ok(mut forwards) = FORWARDS.lock() {
        *forwards = pairs.to_vec();
    }
    Forwarding
}
/// Capture acceptance runs: QEMU's `none` backend, whose input is
/// silence at real-time rate (the wav backend has no input; the host's
/// microphone is never used), instead of the wav file. Set until dropped.
static SILENT_INPUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub struct SilentInput;
impl Drop for SilentInput {
    fn drop(&mut self) {
        SILENT_INPUT.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}
pub fn silent_input() -> SilentInput {
    SILENT_INPUT.store(true, std::sync::atomic::Ordering::Relaxed);
    SilentInput
}
/// P15: a modern VirtIO sound device (one output and one input stream).
/// `wav` records what the guest played into that file (acceptance runs);
/// without it the host's speakers play it (DirectSound on the build host).
pub fn audio_arguments(wav: Option<&Path>) -> Result<[String; 4], String> {
    let silent = SILENT_INPUT.load(std::sync::atomic::Ordering::Relaxed);
    let backend = match wav {
        _ if silent => "none,id=door-snd,out.frequency=48000,in.frequency=48000".into(),
        Some(path) => format!(
            "wav,id=door-snd,out.frequency=48000,path={}",
            path.to_str().ok_or("sound capture path")?
        ),
        None => "dsound,id=door-snd,out.frequency=48000".into(),
    };
    Ok([
        "-audiodev".into(),
        backend,
        "-device".into(),
        "virtio-sound-pci,audiodev=door-snd,disable-legacy=on".into(),
    ])
}
pub fn storage_arguments(profile: &Value, disk: &Path) -> Result<Vec<String>, String> {
    validate_profile(profile)?;
    if !matches!(
        profile["id"].as_str(),
        Some(
            "qemu-storage-x64"
                | "qemu-persistence-x64"
                | "qemu-services-x64"
                | "qemu-windows-x64"
                | "qemu-desktop-whpx-x64"
                | "qemu-desktop-whpx-smp2-x64"
        )
    ) || profile["data_disk"].is_null()
    {
        return Err("writable data disk requires a declared private storage profile".into());
    }
    Ok(vec![
        "-drive".into(),
        format!("if=none,id=door-data,format=raw,file={}", escaped(disk)?),
        "-device".into(),
        format!(
            "virtio-blk-pci,drive=door-data,disable-legacy=on,serial={}",
            profile["data_disk"]["serial"]
                .as_str()
                .ok_or("data disk serial")?
        ),
    ])
}
#[derive(Debug, Eq, PartialEq)]
pub enum Outcome {
    Interrupted,
    ToolError,
    OutputLimit,
    Timeout,
    CleanupFailed,
    KernelPanic,
    GuestFailure,
    StartupFailure,
    AbnormalExit,
    Incomplete,
    Passed,
}
pub fn classify(
    exit: Option<i32>,
    timeout: bool,
    tool_error: bool,
    output_limit: bool,
    cleanup_failed: bool,
    serial: &str,
    token: &str,
) -> Outcome {
    if tool_error {
        return Outcome::ToolError;
    }
    if cleanup_failed {
        return Outcome::CleanupFailed;
    }
    if output_limit {
        return Outcome::OutputLimit;
    }
    if timeout {
        return Outcome::Timeout;
    }
    let prefix = format!("DOOR_TEST v1 {token} ");
    let mut handoff = false;
    let mut diagnostic = false;
    for line in serial.lines() {
        if let Some(record) = line.strip_prefix(&prefix) {
            match record {
                "POST_EXIT_BOOT_SERVICES" if !handoff && !diagnostic => handoff = true,
                "KERNEL_DIAGNOSTIC PASS" if handoff && !diagnostic => diagnostic = true,
                "KERNEL_PANIC" => return Outcome::KernelPanic,
                "FAIL" => return Outcome::GuestFailure,
                _ => return Outcome::GuestFailure,
            }
        }
    }
    if exit != Some(33) {
        return if handoff {
            Outcome::AbnormalExit
        } else {
            Outcome::StartupFailure
        };
    }
    if handoff && diagnostic {
        Outcome::Passed
    } else {
        Outcome::Incomplete
    }
}
fn outcome(observation: &Observation, serial: &str, token: &str) -> Outcome {
    classify(
        observation.exit_code,
        observation.timed_out,
        observation.tool_error,
        observation.output_limit_exceeded,
        observation.cleanup_failed,
        serial,
        token,
    )
}
pub fn acquired_inputs(profile: &Value) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    validate_profile(profile)?;
    let qemu = acquired_file(profile, "qemu", "path", "sha256")?;
    let code = acquired_file(profile, "firmware", "code_path", "code_sha256")?;
    let template = acquired_file(profile, "firmware", "vars_template", "vars_template_sha256")?;
    Ok((qemu, code, template))
}
pub fn preflight(
    run: &mut Run,
    profile_path: &str,
) -> Result<(Value, PathBuf, PathBuf, PathBuf), String> {
    let profile = image::read_json(&paths::existing(&run.root, profile_path)?)?;
    let (qemu, code, template) = acquired_inputs(&profile)?;
    let version = run.required(qemu.to_str().ok_or("qemu path")?, &["--version"])?;
    if version
        .stdout
        .strip_prefix("QEMU emulator version ")
        .and_then(|text| text.split_whitespace().next())
        != profile["qemu_version"].as_str()
    {
        return Err("QEMU version differs from profile".into());
    }
    for (switch, key) in [
        ("-machine", "machine_version"),
        ("-cpu", "cpu_model"),
        ("-accel", "accelerator"),
    ] {
        let supported = run.required(qemu.to_str().ok_or("qemu path")?, &[switch, "help"])?;
        if !supported
            .stdout
            .split_whitespace()
            .any(|word| Some(word) == profile[key].as_str())
        {
            return Err(format!("profile {key} not supported by acquired QEMU"));
        }
    }
    Ok((profile, qemu, code, template))
}

pub fn command(run: &mut Run, profile_path: &str, image_path: &str) -> Result<(), String> {
    let result = scenario(run, profile_path, image_path, "normal")?;
    if result != Outcome::Passed {
        return Err(format!("guest run {result:?}"));
    }
    Ok(())
}
pub struct Launch {
    pub profile: Value,
    pub qemu: PathBuf,
    pub private: PathBuf,
    pub vars: PathBuf,
    pub serial: PathBuf,
    pub image_record: Value,
    pub args: Vec<String>,
}
/// Owner-approved 24 October 2026: a finished session's `esp/` is a
/// hash-verified copy of an immutable staged image (kept under its run), so
/// it is retired when the next session is prepared; the session keeps
/// `image.json` (those hashes), `serial.log` and `vars.fd`, and gains
/// `esp-retired.json`. Nothing is retired while any QEMU runs, and a session
/// is retired only once it cannot be starting: `prepare_named` records the
/// launching xtask in `owner.json` before staging anything, and that process
/// must have exited (or be this one, whose launches are sequential, so its
/// earlier sessions' QEMU exited). Sessions from older builds without an
/// owner record are retired only after `UNOWNED_AGE`.
fn retire_finished_esps(root: &Path, current: &str, source: &Path) {
    if !matches!(crate::cache::qemu_active(), Ok(false)) {
        return;
    }
    let Ok(running) = running_pids() else {
        return;
    };
    retire_sessions(
        root,
        current,
        Some(source),
        std::process::id(),
        &running,
        std::time::SystemTime::now(),
    );
}
/// Owner record written first into each new session directory.
const OWNER_FILE: &str = "owner.json";
/// Prepare-to-spawn takes seconds; an unowned session this old is dead.
const UNOWNED_AGE: Duration = Duration::from_secs(10 * 60);
/// Whether a session whose launcher is `owner` (from `owner.json`) has
/// certainly finished, given the running process IDs and no running QEMU.
fn launcher_finished(
    owner: Option<u32>,
    me: u32,
    running: &std::collections::HashSet<u32>,
    age: Option<Duration>,
) -> bool {
    match owner {
        // Never this process: its later gate checks re-hash the private
        // copies of sessions it launched (boot gate, desktop validation).
        Some(pid) => pid != me && !running.contains(&pid),
        None => age.is_some_and(|age| age >= UNOWNED_AGE),
    }
}
fn session_owner(session: &Path) -> Option<u32> {
    let record: Value = serde_json::from_slice(&fs::read(session.join(OWNER_FILE)).ok()?).ok()?;
    u32::try_from(record["xtask_pid"].as_u64()?).ok()
}
/// Process IDs from one `tasklist` snapshot.
fn running_pids() -> Result<std::collections::HashSet<u32>, String> {
    #[cfg(windows)]
    {
        use std::{
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        let output = Command::new("tasklist.exe")
            .args(["/FO", "CSV", "/NH"])
            .creation_flags(0x0800_0000)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("cannot list processes".into());
        }
        Ok(parse_tasklist(&String::from_utf8_lossy(&output.stdout)))
    }
    #[cfg(not(windows))]
    Err("process listing requires Windows".into())
}
/// `"Image Name","PID",...` CSV rows to their PIDs.
fn parse_tasklist(text: &str) -> std::collections::HashSet<u32> {
    text.lines()
        .filter_map(|line| line.split("\",\"").nth(1)?.parse().ok())
        .collect()
}
/// `source` is the staged image directory the session being prepared copies
/// from; a scenario that reboots from an earlier session's private image
/// needs that session's `esp/`, so it is never retired here.
fn retire_sessions(
    root: &Path,
    current: &str,
    source: Option<&Path>,
    me: u32,
    running: &std::collections::HashSet<u32>,
    now: std::time::SystemTime,
) {
    let Ok(entries) = fs::read_dir(root.join("out/qemu")) else {
        return;
    };
    let source = source.and_then(|s| fs::canonicalize(s).ok());
    for entry in entries.flatten() {
        let session = entry.path();
        if source.is_some() && fs::canonicalize(&session).ok() == source {
            continue;
        }
        let esp = session.join("esp");
        let image = session.join("image.json");
        let age = fs::metadata(&image)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok());
        if entry.file_name().to_string_lossy() == current
            || !image.is_file()
            || !esp.is_dir()
            || paths::reject_links(&session, &esp).is_err()
            || !launcher_finished(session_owner(&session), me, running, age)
        {
            continue;
        }
        if fs::remove_dir_all(&esp).is_ok() {
            let _ = write_json(
                &session.join("esp-retired.json"),
                &json!({"schema_version":1,"record_kind":"esp_copy_retired",
                    "reason":"finished session; staged image retained under its run; owner approval 2026-10-24",
                    "hashes":"image.json"}),
            );
        }
    }
}
pub fn prepare(run: &mut Run, profile_path: &str, image_path: &str) -> Result<Launch, String> {
    let instance = run.id.clone();
    prepare_named(run, profile_path, image_path, &instance)
}
pub fn prepare_named(
    run: &mut Run,
    profile_path: &str,
    image_path: &str,
    instance: &str,
) -> Result<Launch, String> {
    if instance.is_empty()
        || !instance
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("invalid emulator instance identity".into());
    }
    let (profile, qemu, code, template) = preflight(run, profile_path)?;
    let (image_base, image_record) = image::validate_staged(&run.root, image_path)?;
    retire_finished_esps(&run.root, instance, &image_base);
    let private = run.generated_directory(&format!("out/qemu/{instance}"))?;
    // First record in the session: its launcher, so a concurrent retirement
    // never takes the esp of a session that is still being started.
    write_json(
        &private.join(OWNER_FILE),
        &json!({"schema_version":1,"record_kind":"qemu_session_owner",
            "xtask_pid":std::process::id()}),
    )?;
    let vars = private.join("vars.fd");
    fs::copy(&template, &vars).map_err(|e| e.to_string())?;
    if file_hash(&vars)? != file_hash(&template)? {
        return Err("private firmware variables copy changed".into());
    }
    let private_inputs = image_record["entries"]
        .as_array()
        .ok_or("image entries missing")?
        .iter()
        .map(|entry| {
            let destination = entry["path"].as_str().ok_or("image destination missing")?;
            let fixed = match destination {
                "esp/EFI/BOOT/BOOTX64.EFI" => "esp/EFI/BOOT/BOOTX64.EFI",
                "esp/Door/kernel.elf" => "esp/Door/kernel.elf",
                "esp/Door/initfs.tar" => "esp/Door/initfs.tar",
                _ => return Err("unexpected destination".to_string()),
            };
            Ok((
                image_base.join(destination),
                fixed,
                entry["artifact"].clone(),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let private_image = image::stage_inputs(&private, &private_inputs)?;
    write_json(&private.join("image.json"), &private_image)?;
    let serial = private.join("serial.log");
    let token = run.id.clone();
    let disk = private.join("esp");
    let args = arguments(&profile, &code, &vars, &disk, &serial, &token)?;
    Ok(Launch {
        profile,
        qemu,
        private,
        vars,
        serial,
        image_record,
        args,
    })
}
pub fn scenario_exit_code(mode: &str) -> Result<i32, String> {
    if crate::persistence::interruption_stage(mode).is_some() {
        return Ok(37);
    }
    if crate::persistence::is_mode(mode)
        || crate::services::is_mode(mode)
        || crate::windows::is_mode(mode)
    {
        return Ok(33);
    }
    match mode {
        "normal" | "smp-degraded" | "storage-write" | "storage-reopen" => Ok(33),
        "panic" | "bad-handoff" | "hang" | "bad-kernel" | "bad-initfs" | "trap-ud"
        | "trap-double" | "fault-readonly" | "fault-nx" | "fault-guard" => Ok(35),
        _ => Err("unsupported guest scenario".into()),
    }
}
pub fn scenario(
    run: &mut Run,
    profile_path: &str,
    image_path: &str,
    mode: &str,
) -> Result<Outcome, String> {
    scenario_with_disk(run, profile_path, image_path, mode, None)
}
pub fn scenario_with_disk(
    run: &mut Run,
    profile_path: &str,
    image_path: &str,
    mode: &str,
    data_disk: Option<&str>,
) -> Result<Outcome, String> {
    let expected_exit = scenario_exit_code(mode)?;
    if (matches!(mode, "storage-write" | "storage-reopen")
        || crate::persistence::is_mode(mode)
        || crate::services::is_mode(mode)
        || crate::windows::is_mode(mode))
        != data_disk.is_some()
    {
        return Err("storage modes require exactly one private data disk".into());
    }
    let Launch {
        profile,
        qemu,
        private,
        vars,
        serial,
        image_record,
        mut args,
    } = prepare(run, profile_path, image_path)?;
    let token = run.id.clone();
    let disk = private.join("esp");
    let data_before = if let Some(name) = data_disk {
        let path = crate::storage::confined(&run.root, name)?;
        let audit = if matches!(mode, "windows-base" | "windows-reopen") {
            crate::services::inspect_windows(&path).or_else(|_| crate::services::inspect(&path))?
        } else if crate::windows::is_mode(mode) {
            crate::services::inspect(&path)?
        } else if crate::services::is_mode(mode) {
            crate::services::inspect_mode(&path, mode)?
        } else if crate::persistence::is_mode(mode) {
            crate::persistence::inspect(&path)?
        } else {
            crate::storage::inspect(&path, mode == "storage-reopen")?
        };
        if (profile_path == crate::persistence::PROFILE) != crate::persistence::is_mode(mode)
            || (profile_path == crate::services::PROFILE) != crate::services::is_mode(mode)
            || (profile_path == crate::windows::PROFILE) != (crate::windows::is_mode(mode))
        {
            return Err("disk mode/profile mismatch".into());
        }
        args.extend(storage_arguments(&profile, &path)?);
        Some(audit)
    } else {
        if !profile["data_disk"].is_null() {
            return Err("storage profile requires data disk".into());
        }
        None
    };
    args.extend([
        "-fw_cfg".into(),
        format!("name=opt/door/mode,string={mode}"),
    ]);
    let observed = run.watched(
        qemu.to_str().ok_or("qemu path")?,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        Duration::from_secs(30),
        expected_exit,
        &[(&serial, 1024 * 1024)],
    )?;
    if fs::metadata(&serial).is_ok_and(|metadata| metadata.len() > 1024 * 1024) {
        return Err("guest serial exceeds 1 MiB evidence limit".into());
    }
    let bytes = if serial.is_file() {
        fs::read(&serial).map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    let captured = String::from_utf8_lossy(&bytes);
    let result = if crate::persistence::interruption_stage(mode).is_some()
        && observed.exit_code == Some(37)
        && !observed.timed_out
        && !observed.tool_error
        && !observed.output_limit_exceeded
        && !observed.cleanup_failed
        && crate::persistence::interrupted(&captured, &token, mode)
    {
        Outcome::Interrupted
    } else {
        outcome(&observed, &captured, &token)
    };
    image::validate_staged(
        &run.root,
        &relative(&run.root, &private.join("image.json"))?,
    )?;
    acquired_file(&profile, "qemu", "path", "sha256")?;
    acquired_file(&profile, "firmware", "code_path", "code_sha256")?;
    acquired_file(
        &profile,
        "firmware",
        "vars_template",
        "vars_template_sha256",
    )?;
    let data_evidence = if let Some(name) = data_disk {
        let path = crate::storage::confined(&run.root, name)?;
        let snapshot = private.join("data-after.raw");
        fs::copy(&path, &snapshot).map_err(|e| e.to_string())?;
        let after = if matches!(mode, "windows-base" | "windows-reopen") {
            crate::services::inspect_windows(&snapshot)
        } else if crate::windows::is_mode(mode) {
            crate::services::inspect(&snapshot)
        } else if crate::services::is_mode(mode) {
            crate::services::inspect_mode(&snapshot, mode)
        } else if crate::persistence::is_mode(mode) {
            crate::persistence::inspect(&snapshot)
        } else {
            crate::storage::inspect(&snapshot, true)
        };
        json!({"path":name,"before":data_before,"after":{"path":relative(&run.root,&snapshot)?,"sha256":file_hash(&snapshot)?,"audit":after.as_ref().ok()},"audit_error":after.err()})
    } else {
        Value::Null
    };
    let record = json!({"schema_version":1,"record_kind":"guest_run","outcome":format!("{result:?}"),"scenario":mode,"data_disk":data_evidence,"boot_medium":{"path":relative(&run.root,&disk)?,"kind":"read-only-vvfat-via-virtio"},"run_token":token,"profile":profile,
        "image":image_record,"argv":args,"private_vars":relative(&run.root,&vars)?,"private_vars_sha256":file_hash(&vars)?,"serial":{"path":relative(&run.root,&serial)?,"sha256":if serial.is_file(){Some(file_hash(&serial)?)}else{None}},
        "verification_state":if result==Outcome::Passed{"guest_tested"}else{"attempted"}});
    run.record_guest(&record)?;
    if matches!(result, Outcome::Passed | Outcome::Interrupted)
        && data_disk.is_some()
        && !record["data_disk"]["audit_error"].is_null()
    {
        return Err(format!(
            "guest completed but independent storage audit failed: {}",
            record["data_disk"]["audit_error"]
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_sessions_whose_launcher_exited_lose_their_esp() {
        use std::{collections::HashSet, time::SystemTime};
        let workspace = crate::test_support::Workspace::new();
        let root = &workspace.root;
        let me = std::process::id();
        let running: HashSet<u32> = [me, 4242].into_iter().collect();
        let old = Duration::from_secs(3600);
        let session = |name: &str, owner: Option<u32>, staged: bool| {
            let dir = root.join("out/qemu").join(name);
            fs::create_dir_all(dir.join("esp/EFI")).unwrap();
            if let Some(pid) = owner {
                write_json(&dir.join(OWNER_FILE), &json!({"xtask_pid": pid})).unwrap();
            }
            if staged {
                write_json(&dir.join("image.json"), &json!({})).unwrap();
            }
            dir
        };
        let finished = session("finished", Some(9_999_999), true);
        let starting = session("starting", Some(4242), true);
        let ours = session("ours", Some(me), true);
        let current = session("current", Some(me), true);
        let staging = session("staging", Some(9_999_999), false);
        let legacy = session("legacy", None, true);
        retire_sessions(root, "current", None, me, &running, SystemTime::now());
        let kept = |dir: &PathBuf| dir.join("esp").is_dir();
        assert!(!kept(&finished) && finished.join("esp-retired.json").is_file());
        assert!(
            kept(&ours),
            "this process re-validates its own sessions later"
        );
        assert!(kept(&starting), "launcher still running: may be starting");
        assert!(kept(&current) && kept(&staging));
        assert!(kept(&legacy), "an unowned recent session may be starting");
        retire_sessions(root, "current", None, me, &running, SystemTime::now() + old);
        assert!(!kept(&legacy) && kept(&starting));
        assert!(launcher_finished(None, me, &running, Some(UNOWNED_AGE)));
        assert!(!launcher_finished(None, me, &running, None));
        assert_eq!(
            parse_tasklist(
                "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\"qemu-system-x86_64.exe\",\"4242\",\"Console\",\"1\",\"1,024 K\"\r\nINFO: none\r\n"
            ),
            [0, 4242].into_iter().collect()
        );
    }
    fn resolved() -> Value {
        json!({"schema_version":1,"record_kind":"hardware_profile","architecture":"amd64","firmware_kind":"uefi","machine_version":"pc-q35-10.2","accelerator":"tcg","cpu_model":"qemu64","qemu_version":"10.2.0","vcpus":1,"memory_mib":2048,"network":{"enabled":false},"physical_disk_passthrough":false,"qemu":{"path":"tool","sha256":"a".repeat(64)},"firmware":{"code_path":"code","code_sha256":"b".repeat(64),"vars_template":"vars","vars_template_sha256":"c".repeat(64)}})
    }
    #[test]
    fn profile_rejects_missing_inputs_injection_network_and_unpinned_machine() {
        for (field, value) in [
            ("machine_version", json!("q35")),
            ("cpu_model", json!("qemu64,feature=on")),
            ("accelerator", json!("whpx")),
            ("qemu_version", Value::Null),
        ] {
            let mut p = resolved();
            p[field] = value;
            assert!(validate_profile(&p).is_err());
        }
        let mut p = resolved();
        p["network"]["enabled"] = json!(true);
        assert!(validate_profile(&p).is_err());
        let mut p = resolved();
        p["firmware"]["vars_template"] = Value::Null;
        assert!(validate_profile(&p).is_err());
    }
    #[test]
    fn hardware_desktop_profile_cannot_change_diagnostic_or_storage_boundaries() {
        let mut p = resolved();
        p["accelerator"] = json!("whpx");
        assert!(validate_profile(&p).is_err());
        p["id"] = json!("qemu-desktop-whpx-x64");
        p["display"] = json!({"device":"ramfb"});
        let args = arguments(
            &p,
            Path::new("code"),
            Path::new("vars"),
            Path::new("esp"),
            Path::new("serial"),
            "whpx-test",
        )
        .unwrap();
        assert!(args.windows(2).any(|pair| pair == ["-accel", "whpx"]));
        for (field, value) in [
            ("vcpus", json!(2)),
            ("memory_mib", json!(256)),
            ("accelerator", json!("whpx,kernel-irqchip=off")),
            ("id", json!("qemu-services-x64")),
            ("accelerator", json!("tcg")),
        ] {
            let mut bad = p.clone();
            bad[field] = value;
            assert!(validate_profile(&bad).is_err());
        }
        p["physical_disk_passthrough"] = json!(true);
        assert!(validate_profile(&p).is_err());
    }
    #[test]
    fn exact_arguments_escape_qemu_option_paths_and_disable_network() {
        let args = arguments(
            &resolved(),
            Path::new("code, name.fd"),
            Path::new("vars.fd"),
            Path::new("esp"),
            Path::new("serial.log"),
            "case-123",
        )
        .unwrap();
        assert!(args.iter().any(|arg| arg.contains("file=code,, name.fd")));
        assert!(args.windows(2).any(|pair| pair == ["-nic", "none"]));
    }
    #[test]
    fn smp_profiles_require_their_exact_identity_and_emit_the_requested_cpu_count() {
        for cpus in [2, 4] {
            let mut profile = resolved();
            profile["vcpus"] = json!(cpus);
            assert!(validate_profile(&profile).is_err());
            profile["id"] = json!(format!("qemu-smp{cpus}-x64"));
            let args = arguments(
                &profile,
                Path::new("code"),
                Path::new("vars"),
                Path::new("esp"),
                Path::new("serial"),
                "smp-case",
            )
            .unwrap();
            assert!(
                args.windows(2)
                    .any(|pair| pair[0] == "-smp" && pair[1] == cpus.to_string())
            );
            profile["memory_mib"] = json!(256);
            assert!(validate_profile(&profile).is_err());
        }
        for cpus in [0, 3, 8] {
            let mut profile = resolved();
            profile["vcpus"] = json!(cpus);
            assert!(validate_profile(&profile).is_err());
        }
        assert_eq!(scenario_exit_code("smp-degraded"), Ok(33));
        assert_eq!(scenario_exit_code("trap-double"), Ok(35));
        assert!(scenario_exit_code("future-mode").is_err());
    }
    #[test]
    fn guest_pass_needs_bound_ordered_kernel_records_and_exact_exit() {
        let good =
            "DOOR_TEST v1 case POST_EXIT_BOOT_SERVICES\nDOOR_TEST v1 case KERNEL_DIAGNOSTIC PASS\n";
        assert_eq!(
            classify(Some(33), false, false, false, false, good, "case"),
            Outcome::Passed
        );
        for text in [
            "PASS",
            "DOOR_TEST v1 other POST_EXIT_BOOT_SERVICES",
            "DOOR_TEST v1 case KERNEL_DIAGNOSTIC PASS",
        ] {
            assert_ne!(
                classify(Some(33), false, false, false, false, text, "case"),
                Outcome::Passed
            );
        }
        assert_eq!(
            classify(Some(0), false, false, false, false, good, "case"),
            Outcome::AbnormalExit
        );
        assert_eq!(
            classify(Some(33), true, false, false, false, good, "case"),
            Outcome::Timeout
        );
        assert_eq!(
            classify(
                Some(33),
                false,
                false,
                false,
                false,
                &format!("{good}DOOR_TEST v1 case KERNEL_PANIC"),
                "case"
            ),
            Outcome::KernelPanic
        );
        assert_eq!(
            classify(None, false, true, false, false, "", "case"),
            Outcome::ToolError
        );
    }
}
