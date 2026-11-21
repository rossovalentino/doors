//! Persistent phase qualification verifies retained actual artifacts and guest bytes.
use crate::{evidence::file_hash, image, paths, phase::Phase};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub(crate) fn checked_record(root: &Path, binding: &Value) -> Result<Value, String> {
    let path = paths::existing(root, binding["path"].as_str().ok_or("evidence path")?)?;
    if file_hash(&path)? != binding["sha256"].as_str().ok_or("evidence digest")? {
        return Err("guest evidence digest changed".into());
    }
    image::read_json(&path)
}
pub(crate) fn logs(
    root: &Path,
    report: &Value,
    controlled_exit: Option<(&Value, i32)>,
) -> Result<(), String> {
    if report["inputs_unchanged"] != true {
        return Err("guest source changed during run".into());
    }
    for command in report["commands"].as_array().ok_or("guest commands")? {
        if !command["output_budget_error"].is_null() {
            return Err("guest command exceeded output budget".into());
        }
        // A caller may name its separately validated controlled process. Ordinary
        // prerequisites must exit zero; their own receipt cannot select a policy.
        let expected = controlled_exit
            .filter(|(id, _)| command["id"].as_str().is_some() && command["id"] == **id)
            .map_or(0, |(_, exit)| exit);
        if command["required"] == true
            && !crate::spec::successful_command_at_exit(command, expected)
        {
            return Err("guest required prerequisite failed".into());
        }
        for stream in ["stdout", "stderr"] {
            let path =
                paths::existing(root, command[stream]["path"].as_str().ok_or("stream path")?)?;
            if file_hash(&path)? != command[stream]["sha256"].as_str().ok_or("stream digest")? {
                return Err("guest log changed".into());
            }
        }
    }
    Ok(())
}

pub(crate) fn private_launch(
    root: &Path,
    record: &Value,
    child: &Value,
    staged: &Value,
    profile_path: &str,
) -> Result<Vec<String>, String> {
    let token = child["id"].as_str().ok_or("child identity")?;
    if record["run_token"] != token
        || record["boot_medium"]["kind"] != "read-only-vvfat-via-virtio"
        || record["profile"] != image::read_json(&root.join(profile_path))?
    {
        return Err("guest identity/profile differs".into());
    }
    let directory = format!("out/qemu/{token}");
    if record["boot_medium"]["path"] != format!("{directory}/esp")
        || record["private_vars"] != format!("{directory}/vars.fd")
        || record["serial"]["path"] != format!("{directory}/serial.log")
    {
        return Err("private guest paths differ from run".into());
    }
    let (base, private_image) = image::validate_staged(root, &format!("{directory}/image.json"))?;
    if private_image != *staged {
        return Err("private booted image differs".into());
    }
    let vars = paths::existing(root, record["private_vars"].as_str().ok_or("private vars")?)?;
    if file_hash(&vars)?
        != record["private_vars_sha256"]
            .as_str()
            .ok_or("private vars hash")?
    {
        return Err("private firmware variables changed".into());
    }
    let (_, code, _) = crate::emulator::acquired_inputs(&record["profile"])?;
    // Use the same canonical-root paths as preparation. `arguments` owns all
    // option syntax, so altered declarations cannot merely bind arbitrary argv.
    crate::emulator::arguments(
        &record["profile"],
        &code,
        &vars,
        &base.join("esp"),
        &root.join(format!("{directory}/serial.log")),
        token,
    )
}
pub(crate) fn process<'a>(
    child: &'a Value,
    profile: &Value,
    args: &[String],
) -> Result<&'a Value, String> {
    let mut argv = vec![profile["qemu"]["path"].clone()];
    argv.extend(args.iter().map(|v| json!(v)));
    let matches: Vec<_> = child["commands"]
        .as_array()
        .ok_or("guest commands")?
        .iter()
        .filter(|c| c["argv"] == json!(argv))
        .collect();
    if matches.len() != 1 {
        return Err("exact actual QEMU command missing/duplicated".into());
    }
    Ok(matches[0])
}
pub fn validate_current(run: &crate::evidence::Run) -> Result<(), String> {
    let guest = run.directory.join("guest-run.json");
    validate(
        &run.root,
        &json!({"id":run.id,"certified_phase":run.certified_phase,"guest_tests":"guest_tested","checks":run.checks(),"commands":run.commands(),"source_snapshot":run.build_inputs(),"guest_result":{"path":crate::evidence::relative(&run.root,&guest)?,"sha256":file_hash(&guest)?}}),
        run.certified_phase
            .as_deref()
            .ok_or("selected guest phase")?,
    )
}
/// `cargo xtask revalidate boot --report reports/runs/<id>/report.json`: run
/// the guest gate validation again over a retained verify report, so a
/// stale validator shows up without repeating the guest stages. Read only:
/// it never qualifies the retained run.
pub fn revalidate(run: &mut crate::evidence::Run, name: &str) -> Result<(), String> {
    let path = paths::existing(&run.root, name)?;
    let report = crate::image::read_json(&path)?;
    let id = report["id"].as_str().ok_or("retained run id")?;
    if name != format!("reports/runs/{id}/report.json") {
        return Err("retained report path/id differ".into());
    }
    let phase = report["certified_phase"]
        .as_str()
        .ok_or("retained certified phase")?;
    let guest = run.root.join(format!("reports/runs/{id}/guest-run.json"));
    let audit = json!({"id":id,"certified_phase":phase,"guest_tests":"guest_tested",
        "checks":report["checks"],"commands":report["commands"],"source_snapshot":report["source_snapshot"],
        "guest_result":{"path":crate::evidence::relative(&run.root,&guest)?,"sha256":file_hash(&guest)?}});
    validate(&run.root, &audit, phase)?;
    run.check(
        "retained-guest-gate-revalidation",
        json!({"original_report":name,"original_run_id":id,"phase":phase,
            "guest_execution":"retained; no new guest execution in this run","qualifies":false}),
    );
    Ok(())
}
pub fn validate(root: &Path, report: &Value, phase: &str) -> Result<(), String> {
    let selected = Phase::parse(phase)?;
    if report["certified_phase"] != phase {
        return Err("guest report certification differs from selected phase".into());
    }
    let required_cases = crate::scenarios::cases(phase)?;
    if report["guest_tests"] != "guest_tested" {
        return Err("phase requires guest qualification".into());
    }
    let checks = report["checks"].as_array().ok_or("P02 checks")?;
    let rebuild = &checks
        .iter()
        .find(|c| c["id"] == "p02-guest-rebuild" && c["outcome"] == "passed")
        .ok_or("guest rebuild check missing")?["details"];
    let passes = rebuild["artifacts"]
        .as_array()
        .ok_or("guest passes missing")?;
    if passes.len() != 2 {
        return Err("two guest passes required".into());
    }
    for (index, role) in ["firmware", "kernel", "initfs"].into_iter().enumerate() {
        let a = &passes[0][index];
        let b = &passes[1][index];
        if a["path"] == b["path"] || a["artifact"]["sha256"] != b["artifact"]["sha256"] {
            return Err("distinct matching guest artifacts required".into());
        }
        for artifact in [a, b] {
            if artifact["build_inputs"] != report["source_snapshot"] {
                return Err("guest artifact source binding differs".into());
            }
            let path = paths::existing(root, artifact["path"].as_str().ok_or("artifact path")?)?;
            if !path.starts_with(root.join("out/builds"))
                || image::inspect_role(&path, role)? != artifact["artifact"]
            {
                return Err("guest artifact bytes/inspection changed".into());
            }
            if role != "initfs" {
                if role == "kernel" {
                    let map_path = paths::existing(
                        root,
                        artifact["link_map"]["path"]
                            .as_str()
                            .ok_or("kernel link map missing")?,
                    )?;
                    if file_hash(&map_path)?
                        != artifact["link_map"]["sha256"]
                            .as_str()
                            .ok_or("kernel map digest")?
                    {
                        return Err("kernel link map changed".into());
                    }
                }
                let (package, feature, target) = if role == "firmware" {
                    ("door-uefi", "firmware", "x86_64-unknown-uefi")
                } else {
                    ("door-kernel", "baremetal", "x86_64-unknown-none")
                };
                let directory = path
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .ok_or("target directory")?;
                let argv = json!([
                    "cargo",
                    "build",
                    "--locked",
                    "--release",
                    "--package",
                    package,
                    "--features",
                    feature,
                    "--target",
                    target,
                    "--target-dir",
                    directory.to_str().ok_or("build directory")?
                ]);
                if !report["commands"]
                    .as_array()
                    .ok_or("build commands")?
                    .iter()
                    .any(|c| {
                        c["argv"] == argv
                            && c["required"] == true
                            && c["outcome"] == "passed"
                            && c["exit_code"] == 0
                            && c["timed_out"] == false
                            && c["output_limit_exceeded"] == false
                            && c["cleanup_error"].is_null()
                            && c["spawn_error"].is_null()
                    })
                {
                    return Err("guest build command missing or failed".into());
                }
            }
        }
    }
    if selected >= Phase::P05 {
        validate_native_artifacts(root, report, passes)?;
    }
    if selected >= Phase::P06 {
        crate::storage::validate_sources(report)?;
    }
    if selected >= Phase::P07 {
        crate::persistence::validate_sources(report)?;
    }
    if selected >= Phase::P08 {
        crate::services::validate_sources(report)?;
    }
    let suite = checked_record(root, &report["guest_result"])?;
    if selected >= Phase::P08 && suite["service_demo"].is_null() {
        return Err("P08 requires actual interactive service-console evidence".into());
    }
    let gate = &checks
        .iter()
        .find(|c| {
            c["id"] == crate::scenarios::check_id(phase).unwrap_or("") && c["outcome"] == "passed"
        })
        .ok_or("boot check missing")?["details"];
    if suite != *gate || suite["outcome"] != "Passed" {
        return Err("boot suite binding differs".into());
    }
    let (_, staged) = image::validate_staged(root, suite["image"].as_str().ok_or("boot image")?)?;
    for (index, entry) in staged["entries"]
        .as_array()
        .ok_or("staged entries")?
        .iter()
        .enumerate()
    {
        if entry["artifact"] != passes[0][index]["artifact"] {
            return Err("booted image differs from rebuilt artifact".into());
        }
    }
    let scenarios = suite["scenarios"].as_array().ok_or("guest scenarios")?;
    if scenarios.len() != required_cases.len() {
        return Err("all distinct phase scenarios required".into());
    }
    let mut storage_guests = Vec::new();
    let mut persistence_guests = Vec::new();
    let mut service_guests = Vec::new();
    let mut windows_guests = Vec::new();
    let mut base_guests = Vec::new();
    for case in required_cases {
        let mode = case.mode;
        let expected = format!("{:?}", case.expected);
        let matching: Vec<_> = scenarios
            .iter()
            .filter(|s| s["scenario"] == case.id)
            .collect();
        if matching.len() != 1 {
            return Err("missing/duplicate guest scenario".into());
        }
        let scenario = matching[0];
        let child = checked_record(root, &scenario["report"])?;
        logs(root, &child, None)?;
        if child["source_snapshot"] != report["source_snapshot"] {
            return Err("guest did not run qualifying source".into());
        }
        let guest = checked_record(root, &scenario["guest"])?;
        if child["guest_result"]["path"] != scenario["guest"]["path"]
            || child["guest_result"]["sha256"] != scenario["guest"]["sha256"]
            || guest["scenario"] != mode
            || guest["image"] != staged
            || guest["outcome"] != expected
            || scenario["expected"] != expected
            || scenario["observed"] != expected
        {
            return Err("guest scenario/image/result binding differs".into());
        }
        let mut args = private_launch(root, &guest, &child, &staged, case.profile)?;
        if matches!(
            case.profile,
            crate::storage::PROFILE
                | crate::persistence::PROFILE
                | crate::services::PROFILE
                | crate::windows::PROFILE
        ) {
            let name = guest["data_disk"]["path"]
                .as_str()
                .ok_or("storage guest has no data image")?;
            args.extend(crate::emulator::storage_arguments(
                &guest["profile"],
                &crate::storage::confined(root, name)?,
            )?);
            if case.profile == crate::storage::PROFILE {
                storage_guests.push(guest.clone());
            } else if case.profile == crate::services::PROFILE {
                service_guests.push(guest.clone());
            } else if case.profile == crate::windows::PROFILE {
                if case.mode == crate::windows::MODE {
                    windows_guests.push(guest.clone());
                } else {
                    base_guests.push(guest.clone());
                }
            } else {
                persistence_guests.push(guest.clone());
            }
        } else if !guest["data_disk"].is_null() {
            return Err("unexpected data disk in historical scenario".into());
        }
        args.extend([
            "-fw_cfg".into(),
            format!("name=opt/door/mode,string={mode}"),
        ]);
        if guest["argv"] != json!(args) {
            return Err("guest launch arguments differ".into());
        }
        let serial_path =
            paths::existing(root, guest["serial"]["path"].as_str().ok_or("serial path")?)?;
        if file_hash(&serial_path)? != guest["serial"]["sha256"].as_str().ok_or("serial digest")? {
            return Err("serial evidence changed".into());
        }
        let serial = fs::read_to_string(&serial_path).map_err(|e| e.to_string())?;
        if scenario["marker"] != case.markers[0]
            || (phase != "P02" && scenario["markers"] != json!(case.markers))
            || !crate::scenarios::serial_matches(phase, &case.markers, &serial)
        {
            return Err("expected guest marker missing".into());
        }
        if selected >= Phase::P05
            && case
                .markers
                .iter()
                .any(|marker| marker.starts_with("DOOR_P05 "))
            && matches!(
                case.expected,
                crate::emulator::Outcome::Passed | crate::emulator::Outcome::Interrupted
            )
        {
            let segments = passes[0][3]["artifact"]["inspection"]["segments"]
                .as_array()
                .ok_or("native runtime segment inventory")?
                .iter()
                .filter(|segment| segment["kind"] == 1)
                .count();
            if crate::scenarios::rollback_stages(&serial) != Some(10 + 2 * segments as u64) {
                return Err(
                    "startup rollback stages differ from the booted native runtime segments".into(),
                );
            }
        }
        let process = process(&child, &guest["profile"], &args)?;
        if process["timeout_ms"] != 30000
            || process["expected_exit_code"] != crate::emulator::scenario_exit_code(mode)?
        {
            return Err("guest process policy differs".into());
        }
        let result = if selected >= Phase::P07
            && process["exit_code"] == 37
            && process["timed_out"] == false
            && process["spawn_error"].is_null()
            && process["output_limit_exceeded"] == false
            && process["cleanup_error"].is_null()
            && crate::persistence::interrupted(
                &serial,
                guest["run_token"].as_str().ok_or("run token")?,
                mode,
            ) {
            crate::emulator::Outcome::Interrupted
        } else {
            crate::emulator::classify(
                process["exit_code"].as_i64().map(|v| v as i32),
                process["timed_out"] == true,
                !process["spawn_error"].is_null(),
                process["output_limit_exceeded"] == true,
                !process["cleanup_error"].is_null(),
                &serial,
                guest["run_token"].as_str().ok_or("run token")?,
            )
        };
        if format!("{result:?}") != expected {
            return Err("retained guest evidence does not classify as expected".into());
        }
    }
    if selected >= Phase::P06 {
        let run_id = report["id"]
            .as_str()
            .ok_or("storage suite run identity missing")?;
        if suite["storage"]["path"] != format!("out/storage/{run_id}/data.raw") {
            return Err("storage fixture must belong to the qualifying run".into());
        }
        crate::storage::validate_suite(root, &suite, &storage_guests)?;
        if selected >= Phase::P07 {
            crate::persistence::session::validate(root, run_id, &suite, &persistence_guests)?;
        }
        if selected >= Phase::P08 {
            crate::services::session::validate(root, run_id, &suite, &service_guests)?;
            crate::services::demo::validate(root, report, &suite, &staged)?;
        }
        if selected >= Phase::P09 {
            crate::windows::gate::validate(root, report, &suite, passes, &windows_guests)?;
        }
        if selected >= Phase::P10 {
            crate::windows::base_verify::validate(root, report, &suite, passes, &base_guests)?;
        }
    }
    let demo = checked_record(root, &suite["demo"])?;
    let demo_report = checked_record(root, &suite["demo"]["report"])?;
    logs(root, &demo_report, None)?;
    if demo_report["success"] != true
        || demo_report["source_snapshot"] != report["source_snapshot"]
        || demo["state"] != "completed"
        || demo["interactions"].as_array().is_none_or(|v| v.len() != 4)
    {
        return Err("demo interaction evidence incomplete".into());
    }
    let base = private_launch(
        root,
        &demo,
        &demo_report,
        &staged,
        crate::scenarios::NORMAL_PROFILE,
    )?;
    let port =
        u16::try_from(demo["qmp_port"].as_u64().ok_or("demo port")?).map_err(|e| e.to_string())?;
    if demo["visible"] != false {
        return Err("completed gate demo must be headless".into());
    }
    let args = crate::demo::arguments(
        base,
        false,
        port,
        demo["uuid"].as_str().ok_or("demo UUID")?,
        phase,
    )?;
    if demo["argv"] != json!(args) {
        return Err("demo launch arguments differ".into());
    }
    let process = process(&demo_report, &demo["profile"], &args)?;
    if process["id"] != demo["process_command"]
        || process["pid"] != demo["pid"]
        || process["required"] != true
        || process["outcome"] != "passed"
        || process["exit_code"] != 0
        || process["expected_exit_code"] != 0
        || process["lifecycle"] != "completed"
        || process["timeout_ms"] != 90000
        || process["duration_ms"].as_u64().is_none_or(|v| v > 90000)
        || process["timed_out"] != false
        || process["output_limit_exceeded"] != false
        || !process["spawn_error"].is_null()
        || !process["cleanup_error"].is_null()
        || !process["controller_error"].is_null()
    {
        return Err("completed bounded demo process missing or failed".into());
    }
    let screen = paths::existing(
        root,
        demo["screenshot"]["path"]
            .as_str()
            .ok_or("screenshot path")?,
    )?;
    if file_hash(&screen)?
        != demo["screenshot"]["sha256"]
            .as_str()
            .ok_or("screenshot hash")?
    {
        return Err("guest screenshot changed".into());
    }
    let serial_path = paths::existing(
        root,
        demo["serial"]["path"].as_str().ok_or("demo serial path")?,
    )?;
    if file_hash(&serial_path)?
        != demo["serial"]["sha256_at_ready"]
            .as_str()
            .ok_or("demo serial hash")?
    {
        return Err("demo serial changed".into());
    }
    let serial = fs::read_to_string(serial_path).map_err(|e| e.to_string())?;
    for (key, marker) in [
        ("h", "DOOR_DEMO HELP"),
        ("d", "DOOR_DEMO DIAGNOSTICS"),
        ("r", "DOOR_DEMO REBOOT"),
        ("p", "DOOR_DEMO PANIC_SCREEN_READY"),
    ] {
        if !serial.contains(marker)
            || !demo["interactions"]
                .as_array()
                .ok_or("demo interactions")?
                .iter()
                .any(|value| value["key"] == key && value["marker"] == marker)
        {
            return Err("demo interaction marker missing".into());
        }
    }
    if serial.matches("DOOR_DEMO READY").count() < 2 || demo["image"] != staged {
        return Err("demo reboot/image evidence incomplete".into());
    }
    let extra = demo["additional_screens"]
        .as_array()
        .ok_or("demo screens")?;
    if extra.len() != 2 {
        return Err("diagnostic and panic screens required".into());
    }
    for screenshot in extra {
        let path = paths::existing(root, screenshot["path"].as_str().ok_or("screen path")?)?;
        if file_hash(&path)? != screenshot["sha256"].as_str().ok_or("screen hash")? {
            return Err("demo additional screen changed".into());
        }
    }
    if extra[0]["sha256"] == demo["screenshot"]["sha256"]
        || extra[1]["sha256"] == demo["screenshot"]["sha256"]
        || extra[0]["sha256"] == extra[1]["sha256"]
    {
        return Err("three distinct guest screens required".into());
    }
    Ok(())
}

/// Qualify the separately linked native runtime and raw ABI caller, then bind
/// their inspected bytes to the archive actually staged for each build pass.
fn validate_native_artifacts(root: &Path, report: &Value, passes: &[Value]) -> Result<(), String> {
    let phase = Phase::parse(report["certified_phase"].as_str().unwrap_or("P05"))?;
    let inventory: Vec<_> = crate::build::native::inventory(phase).collect();
    // Windows phases add every original DLL (`dlls::NAMES`, the GUI DLLs
    // included since P13), the original programs of both widths
    // (`dlls::PROGRAMS`, which `dlls::build` always builds), plus the console
    // fixture; P10 adds msvcrt and its two CRT fixtures.
    let dlls = crate::windows::dlls::NAMES.len()
        + crate::windows::dlls::NAMES32.len()
        + 2 * crate::windows::dlls::PROGRAMS.len();
    let count = 3
        + inventory.len()
        + if phase >= Phase::P10 {
            dlls + 4
        } else if phase >= Phase::P09 {
            dlls + 1
        } else {
            0
        };
    if passes.len() != 2
        || passes
            .iter()
            .any(|pass| pass.as_array().is_none_or(|v| v.len() != count))
    {
        return Err(format!(
            "selected phase requires {count} artifacts in each of two independent builds"
        ));
    }
    let sources = report["source_snapshot"]["files"]
        .as_array()
        .ok_or("native source identity")?;
    let platform = if phase >= Phase::P08 {
        PLATFORM_INPUTS
    } else {
        &[]
    };
    for required in NATIVE_INPUTS.iter().chain(platform) {
        let matching = sources
            .iter()
            .filter(|file| file["path"] == *required)
            .collect::<Vec<_>>();
        if matching.len() != 1
            || matching[0]["sha256"]
                .as_str()
                .is_none_or(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(format!(
                "native source identity missing/duplicated: {required}"
            ));
        }
    }
    let commands = report["commands"]
        .as_array()
        .ok_or("native build commands")?;
    for pass in passes {
        let archive_path =
            paths::existing(root, pass[2]["path"].as_str().ok_or("native initfs path")?)?;
        let bytes = fs::read(archive_path).map_err(|e| e.to_string())?;
        let archive =
            door_initfs::Archive::parse(&bytes).map_err(|e| format!("native initfs {e:?}"))?;
        for (offset, artifact) in inventory.iter().enumerate() {
            let name = artifact.name;
            let entry = &pass[offset + 3];
            let destination = format!("bin/{name}.elf");
            if entry["name"] != name
                || entry["initfs_destination"] != destination
                || entry["build_inputs"] != report["source_snapshot"]
            {
                return Err("native artifact source/name binding differs".into());
            }
            if entry["runtime"] != artifact.runtime {
                return Err("native runtime identity differs".into());
            }
            let path =
                paths::existing(root, entry["path"].as_str().ok_or("native artifact path")?)?;
            if !path.starts_with(root.join("out/builds"))
                || crate::artifact::inspect(&path, "native-user")? != entry["artifact"]
            {
                return Err("native executable inspection changed".into());
            }
            let native = fs::read(&path).map_err(|e| e.to_string())?;
            if archive.file(&destination) != Some(native.as_slice()) {
                return Err(
                    "initfs does not contain the independently built native executable".into(),
                );
            }
            let directory = path
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .ok_or("native build directory")?;
            if path != directory.join(format!("x86_64-unknown-none/release/{name}")) {
                return Err("native executable is outside its target layout".into());
            }
            let map = paths::existing(
                root,
                entry["link_map"]["path"]
                    .as_str()
                    .ok_or("native link map")?,
            )?;
            if map != directory.join(format!("{name}.map"))
                || file_hash(&map)? != entry["link_map"]["sha256"]
            {
                return Err("native link map binding changed".into());
            }
            let flags = crate::build::native::flags(root, directory, artifact);
            let argv = json!([
                "cargo",
                "build",
                "--locked",
                "--release",
                "--package",
                artifact.package,
                "--features",
                "native",
                "--bin",
                name,
                "--target",
                "x86_64-unknown-none",
                "--target-dir",
                directory.to_str().ok_or("native target path")?
            ]);
            let matches = commands
                .iter()
                .filter(|command| command["argv"] == argv)
                .collect::<Vec<_>>();
            if matches.len() != 1 || !successful_native_command(matches[0], &flags) {
                return Err("independent native build command or flags missing/failed".into());
            }
        }
    }
    for (first, second) in passes[0].as_array().unwrap()[3..]
        .iter()
        .zip(&passes[1].as_array().unwrap()[3..])
    {
        if first["path"] == second["path"]
            || first["artifact"]["sha256"] != second["artifact"]["sha256"]
        {
            return Err("native controlled rebuild did not independently match".into());
        }
    }
    if passes[0][3]["artifact"]["sha256"] == passes[0][4]["artifact"]["sha256"] {
        return Err("raw ABI probe is not a distinct native artifact".into());
    }
    Ok(())
}

fn successful_native_command(command: &Value, flags: &str) -> bool {
    command["required"] == true
        && command["outcome"] == "passed"
        && command["exit_code"] == 0
        && command["expected_exit_code"] == 0
        && command["timeout_ms"] == 120000
        && command["timed_out"] == false
        && command["output_limit_exceeded"] == false
        && command["cleanup_error"].is_null()
        && command["spawn_error"].is_null()
        && command["environment_overrides"] == json!([["CARGO_ENCODED_RUSTFLAGS", flags]])
}

const NATIVE_INPUTS: &[&str] = &[
    "Cargo.lock",
    "runtime/abi/src/lib.rs",
    "runtime/abi/src/wire.rs",
    "runtime/abi/src/startup.rs",
    "runtime/sys/src/lib.rs",
    "runtime/rt/src/lib.rs",
    "specs/abi/native-x64-v1.md",
    "tests/fixtures/native/src/runtime.rs",
    "tests/fixtures/native/src/probe.rs",
    "tests/fixtures/native/src/probe.S",
    "tests/fixtures/native/linker.ld",
    "profiles/build/initfs.json",
];

const PLATFORM_INPUTS: &[&str] = &[
    "specs/abi/native-platform-v1.md",
    "profiles/build/initfs-p08.json",
    "services/Cargo.toml",
    "services/linker.ld",
    "tests/fixtures/native-workload/Cargo.toml",
    "tests/fixtures/native-workload/linker.ld",
    "crates/service-protocol/Cargo.toml",
    "frameworks/service-client/Cargo.toml",
    "fs/storage-core/Cargo.toml",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_prerequisites_reject_contradictory_receipts_before_reading_streams() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let good = json!({"required":true,"outcome":"passed","exit_code":0,
            "expected_exit_code":0,"timed_out":false,"output_limit_exceeded":false,
            "spawn_error":null,"cleanup_error":null});
        assert!(crate::spec::successful_command(&good));
        for (field, value) in [
            ("outcome", json!("failed")),
            ("exit_code", json!(1)),
            ("expected_exit_code", json!(1)),
            ("timed_out", json!(true)),
            ("output_limit_exceeded", json!(true)),
            ("spawn_error", json!("spawn failed")),
            ("cleanup_error", json!("cleanup failed")),
            ("output_budget_error", json!("headroom exhausted")),
        ] {
            let mut command = good.clone();
            command[field] = value;
            let report = json!({"inputs_unchanged":true,"commands":[command]});
            assert_eq!(
                logs(root, &report, None),
                Err(if field == "output_budget_error" {
                    "guest command exceeded output budget".into()
                } else {
                    "guest required prerequisite failed".into()
                }),
                "{field}"
            );
        }
    }

    #[test]
    fn controlled_exit_policy_is_explicit_and_cannot_relax_prerequisites() {
        let workspace = crate::test_support::Workspace::new();
        let root = &workspace.root;
        let stream = root.join("receipt.log");
        fs::write(&stream, b"captured").unwrap();
        let binding = json!({"path":"receipt.log","sha256":file_hash(&stream).unwrap()});
        let prerequisite = json!({"id":"preflight","required":true,"outcome":"passed",
            "exit_code":0,"expected_exit_code":0,"timed_out":false,
            "output_limit_exceeded":false,"spawn_error":null,"cleanup_error":null,
            "stdout":binding,"stderr":binding});
        let mut process = prerequisite.clone();
        process["id"] = json!("controlled");
        process["exit_code"] = json!(33);
        process["expected_exit_code"] = json!(33);
        let report = json!({"inputs_unchanged":true,"commands":[prerequisite,process]});
        let id = json!("controlled");
        assert!(logs(root, &report, None).is_err());
        assert!(logs(root, &report, Some((&id, 33))).is_ok());
        assert!(logs(root, &report, Some((&json!("preflight"), 33))).is_err());
        let mut bad = report.clone();
        bad["commands"][0]["exit_code"] = json!(33);
        bad["commands"][0]["expected_exit_code"] = json!(33);
        assert!(logs(root, &bad, Some((&id, 33))).is_err());
        for (field, value) in [
            ("expected_exit_code", json!(0)),
            ("output_limit_exceeded", json!(true)),
            ("cleanup_error", json!("failed")),
        ] {
            let mut bad = report.clone();
            bad["commands"][1][field] = value;
            assert!(logs(root, &bad, Some((&id, 33))).is_err(), "{field}");
        }
    }

    #[test]
    fn native_evidence_binds_two_builds_exact_commands_sources_and_archive_bytes() {
        let workspace = crate::test_support::Workspace::new();
        let root = &workspace.root;
        let snapshot = json!({"files": NATIVE_INPUTS.iter().map(|path| json!({"path":path,"sha256":"a".repeat(64)})).collect::<Vec<_>>()});
        let mut commands = Vec::new();
        let mut passes = Vec::new();
        for pass in 0..2 {
            let directory =
                paths::fresh_directory(root, &format!("out/builds/test/pass-{pass}")).unwrap();
            fs::create_dir_all(directory.join("x86_64-unknown-none/release")).unwrap();
            let archive_path = directory.join("initfs.tar");
            let mut artifacts = vec![
                Value::Null,
                Value::Null,
                json!({"path":crate::evidence::relative(root,&archive_path).unwrap()}),
            ];
            let mut archive = Vec::new();
            for (index, name) in ["native-runtime", "native-probe"].iter().enumerate() {
                let mut bytes = crate::image_fixtures::elf();
                crate::image_fixtures::put64(&mut bytes, 24, 0x400000);
                crate::image_fixtures::put64(&mut bytes, 80, 0x400000);
                bytes[4096] = index as u8;
                let path = directory.join(format!("x86_64-unknown-none/release/{name}"));
                let map = directory.join(format!("{name}.map"));
                fs::write(&path, &bytes).unwrap();
                fs::write(&map, name).unwrap();
                let destination = format!("bin/{name}.elf");
                let mut header = [0; 512];
                door_initfs::encode_header(&mut header, &destination, bytes.len(), false).unwrap();
                archive.extend_from_slice(&header);
                archive.extend_from_slice(&bytes);
                artifacts.push(json!({"name":name,"path":crate::evidence::relative(root,&path).unwrap(),
                    "artifact":crate::artifact::inspect(&path,"native-user").unwrap(),"build_inputs":snapshot,
                    "initfs_destination":destination,"link_map":{"path":crate::evidence::relative(root,&map).unwrap(),"sha256":file_hash(&map).unwrap()},
                    "runtime":if index==0{"N0 door-rt/door-sys"}else{"independent raw assembly ABI literals"}}));
                let flags = [
                    format!("--remap-path-prefix={}=/door-os", root.display()),
                    "-C".into(),
                    "code-model=small".into(),
                    "-C".into(),
                    "lto=off".into(),
                    "-C".into(),
                    "relocation-model=static".into(),
                    "-C".into(),
                    "no-redzone=yes".into(),
                    "-C".into(),
                    format!(
                        "link-arg=-T{}",
                        root.join("tests/fixtures/native/linker.ld").display()
                    ),
                    "-C".into(),
                    "link-arg=--build-id=none".into(),
                    "-C".into(),
                    format!("link-arg=-Map={}", map.display()),
                ]
                .join("\x1f");
                commands.push(json!({"argv":["cargo","build","--locked","--release","--package","door-native-fixtures","--features","native","--bin",name,"--target","x86_64-unknown-none","--target-dir",directory],
                    "required":true,"outcome":"passed","exit_code":0,"expected_exit_code":0,"timeout_ms":120000,"timed_out":false,"output_limit_exceeded":false,
                    "environment_overrides":[["CARGO_ENCODED_RUSTFLAGS",flags]]}));
            }
            archive.extend_from_slice(&[0; 1024]);
            fs::write(archive_path, archive).unwrap();
            passes.push(json!(artifacts));
        }
        let report = json!({"source_snapshot":snapshot,"commands":commands});
        assert!(validate_native_artifacts(root, &report, &passes).is_ok());
        for field in [
            "environment_overrides",
            "argv",
            "required",
            "exit_code",
            "expected_exit_code",
            "timeout_ms",
        ] {
            let mut bad = report.clone();
            bad["commands"][0][field] = Value::Null;
            assert!(
                validate_native_artifacts(root, &bad, &passes).is_err(),
                "accepted missing {field}"
            );
        }
        let mut bad = report.clone();
        bad["commands"].as_array_mut().unwrap().remove(0);
        assert!(validate_native_artifacts(root, &bad, &passes).is_err());
        let mut bad = passes.clone();
        bad[1] = bad[0].clone();
        assert!(validate_native_artifacts(root, &report, &bad).is_err());
        let mut bad = passes.clone();
        bad[0].as_array_mut().unwrap().pop();
        assert!(validate_native_artifacts(root, &report, &bad).is_err());
        let mut bad = report.clone();
        bad["source_snapshot"]["files"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(validate_native_artifacts(root, &bad, &passes).is_err());
        let mut bad = passes.clone();
        bad[0][3]["initfs_destination"] = json!("bin/not-runtime.elf");
        assert!(validate_native_artifacts(root, &report, &bad).is_err());
        let archive = paths::existing(root, passes[0][2]["path"].as_str().unwrap()).unwrap();
        let mut bytes = fs::read(&archive).unwrap();
        bytes[512 + 4096] ^= 1;
        fs::write(archive, bytes).unwrap();
        assert!(validate_native_artifacts(root, &report, &passes).is_err());
    }

    #[test]
    fn private_medium_requires_actual_image_and_exact_launch_identity() {
        let workspace = crate::test_support::Workspace::new();
        let root = &workspace.root;
        for name in ["qemu", "code", "template"] {
            fs::write(root.join(name), name).unwrap();
        }
        let profile = json!({"schema_version":1,"record_kind":"hardware_profile","architecture":"amd64","firmware_kind":"uefi","machine_version":"pc-q35-11.1","accelerator":"tcg","cpu_model":"qemu64-v1","qemu_version":"11.1.0","vcpus":1,"memory_mib":2048,"network":{"enabled":false},"physical_disk_passthrough":false,
            "qemu":{"path":root.join("qemu"),"sha256":file_hash(&root.join("qemu")).unwrap()},"firmware":{"code_path":root.join("code"),"code_sha256":file_hash(&root.join("code")).unwrap(),"vars_template":root.join("template"),"vars_template_sha256":file_hash(&root.join("template")).unwrap()}});
        crate::evidence::write_json(
            &root.join("profiles/hardware/qemu-diagnostic-x64.json"),
            &profile,
        )
        .unwrap();
        fs::write(root.join("firmware.efi"), crate::image_fixtures::pe()).unwrap();
        fs::write(root.join("kernel.elf"), crate::image_fixtures::elf()).unwrap();
        fs::write(root.join("initfs.tar"), crate::initfs::fixture()).unwrap();
        let inputs = [
            ("firmware.efi", "firmware", "esp/EFI/BOOT/BOOTX64.EFI"),
            ("kernel.elf", "kernel", "esp/Door/kernel.elf"),
            ("initfs.tar", "initfs", "esp/Door/initfs.tar"),
        ]
        .into_iter()
        .map(|(file, role, destination)| {
            let path = root.join(file);
            let inspected = image::inspect_role(&path, role).unwrap();
            (path, destination, inspected)
        })
        .collect::<Vec<_>>();
        let private = paths::fresh_directory(root, "out/qemu/case-123").unwrap();
        let staged = image::stage_inputs(&private, &inputs).unwrap();
        crate::evidence::write_json(&private.join("image.json"), &staged).unwrap();
        fs::copy(root.join("template"), private.join("vars.fd")).unwrap();
        let child = json!({"id":"case-123"});
        let record = json!({"run_token":"case-123","profile":profile,"boot_medium":{"path":"out/qemu/case-123/esp","kind":"read-only-vvfat-via-virtio"},"private_vars":"out/qemu/case-123/vars.fd","private_vars_sha256":file_hash(&private.join("vars.fd")).unwrap(),"serial":{"path":"out/qemu/case-123/serial.log"}});
        let args = private_launch(
            root,
            &record,
            &child,
            &staged,
            crate::scenarios::NORMAL_PROFILE,
        )
        .unwrap();
        for (field, value) in [
            ("profile", Value::Null),
            ("run_token", json!("other")),
            (
                "boot_medium",
                json!({"path":"out/missing/esp","kind":"read-only-vvfat-via-virtio"}),
            ),
        ] {
            let mut altered = record.clone();
            altered[field] = value;
            assert!(
                private_launch(
                    root,
                    &altered,
                    &child,
                    &staged,
                    crate::scenarios::NORMAL_PROFILE
                )
                .is_err()
            );
        }
        let mut argv = vec![record["profile"]["qemu"]["path"].clone()];
        argv.extend(args.iter().map(|v| json!(v)));
        let command_child = json!({"commands":[{"argv":argv}]});
        assert!(process(&command_child, &record["profile"], &args).is_ok());
        let mut changed = args.clone();
        changed.push("-enable-kvm".into());
        assert!(process(&command_child, &record["profile"], &changed).is_err());
        fs::remove_file(private.join("esp/Door/kernel.elf")).unwrap();
        assert!(
            private_launch(
                root,
                &record,
                &child,
                &staged,
                crate::scenarios::NORMAL_PROFILE
            )
            .is_err()
        );
    }
}
