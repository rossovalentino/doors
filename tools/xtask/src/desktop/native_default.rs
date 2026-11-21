//! Real automatic-mode startup and title gestures, using only a private disk.
use super::*;
use crate::persistence::{catalog, journal, tree};
use std::path::Path;

const CAPTURES: &[&str] = &[
    "auto-empty",
    "auto-files",
    "auto-max",
    "auto-max-restored",
    "auto-half",
    "auto-half-restored",
];

fn one<'a>(checks: &'a [Value], id: &str) -> Result<&'a Value, String> {
    let values: Vec<_> = checks
        .iter()
        .filter(|v| v["id"] == id && v["outcome"] == "passed")
        .collect();
    if values.len() != 1 {
        return Err(format!("one native-default {id} required"));
    }
    Ok(&values[0]["details"])
}
fn disk(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() != 128 * 1024 * 1024 {
        return Err("native-default disk size".into());
    }
    let journal = journal::inspect(&bytes)?;
    if journal.selected().committed {
        return Err("native-default dirty final journal".into());
    }
    let home = journal.replayed_home(&bytes)?;
    let tree = tree::inspect(&home)?;
    let catalog = catalog::decode(&tree)?.ok_or("native-default catalog missing")?;
    let paths = catalog.paths()?;
    for suffix in [
        "",
        "/Desktop",
        "/Documents",
        "/Downloads",
        "/Music",
        "/Pictures",
        "/Videos",
    ] {
        let node = paths
            .get(&format!("/users/1001{suffix}"))
            .ok_or("native-default user folder missing")?;
        if node.kind != 2 || node.owner != 1001 {
            return Err("native-default user folder identity".into());
        }
    }
    let summary = json!({"revision":catalog.revision,"next_object":catalog.next_object,"next_key":catalog.next_key,
        "paths":paths.len(),"associations":catalog.associations});
    Ok(
        json!({"sha256":file_hash(path)?,"journal":journal.summary(),"tree":tree.summary(),"catalog":summary}),
    )
}
/// File Explorer's light caption: its own tabbed caption in the shared
/// caption role (`document_view::Theme`, BGRA [243, 235, 225]).
const CAPTION_RGB: [u8; 3] = [225, 235, 243];
/// The guest's automatic scale for `width` x `height`: the host scale hint
/// when offered, else the integer density policy (no stored preference).
fn automatic_scale(
    width: u32,
    height: u32,
    hint: Option<u64>,
) -> Result<door_window::graphics::Scale, String> {
    let hint = hint
        .map(u32::try_from)
        .transpose()
        .map_err(|_| "auto scale hint")?
        .unwrap_or(0);
    Ok(door_window::policy::resolve_scale(0, hint, width, height))
}
/// Pixel probes of one capture.
type Probes = Vec<(usize, usize, [u8; 3])>;
/// Probes of each capture, in `CAPTURES` order: Explorer's caption at every
/// stage, and beside each restored or half window a content pixel inside it
/// (white) and the wallpaper of the empty desktop just outside it.
fn expected_probes(
    empty: &scene::Pixels,
    n: &window_points::NativeDefault,
) -> Result<Vec<Probes>, String> {
    let caption = |i: usize| (n.caption[i].0, n.caption[i].1, CAPTION_RGB);
    let beside = |i: usize| -> Result<Vec<_>, String> {
        let (o, b) = (n.outside[i], n.body[i]);
        Ok(vec![
            (b.0, b.1, [255; 3]),
            (o.0, o.1, empty.pixel(o.0, o.1)?),
        ])
    };
    let mut stages = vec![vec![], vec![caption(0)], vec![caption(1)]];
    for (stage, i) in [(2, 0), (3, 1), (4, 2)] {
        let mut probes = vec![caption(stage)];
        probes.extend(beside(i)?);
        stages.push(probes);
    }
    Ok(stages)
}
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk_path = run.directory.join("desktop-native-auto.raw");
    fs::write(&disk_path, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut record =
        session::DesktopOptions::parse(&["--headless".into()])?.launch(run, image, &disk_path)?;
    let result: Result<(), String> = (|| {
        let selected = one(run.checks(), "desktop-native-mode-selection")?["selected"]
            .as_str()
            .ok_or("auto selected mode")?
            .to_owned();
        let (w, h) = selected.split_once('x').ok_or("auto mode")?;
        let (width, height) = (
            w.parse::<u32>().map_err(|_| "auto width")?,
            h.parse::<u32>().map_err(|_| "auto height")?,
        );
        // The guest's automatic scale: the host hint when offered, else the
        // integer density policy (`policy::resolve_scale`, no stored choice).
        let hint = one(run.checks(), "desktop-native-scale-hint")?["hint_percent"].as_u64();
        let scale = automatic_scale(width, height, hint)?;
        let points = window_points::native_default((width, height), scale)?;
        let (lw, lh) = points.logical;
        let mut c = connection(&record)?;
        let mut p = (width as i32 / 2, height as i32 / 2);
        wait_serial(
            &run.root,
            &record,
            &format!(
                "DENSITY physical={selected} logical={lw}x{lh} factor={}",
                door_window::display_scale::factor_text(scale)
            ),
            Duration::from_secs(10),
        )?;
        if serial(&run.root, &record)?.contains("APP_LAUNCHED") {
            return Err("automatic default autostarted apps".into());
        }
        let empty = scene::capture_settled(run, &mut c, "auto-empty", &[])?;
        if (empty.width, empty.height) != (width as usize, height as usize) {
            return Err("automatic native extent".into());
        }
        let expected = expected_probes(&empty, &points)?;
        resolution::activate(&run.root, &record, &mut c, 0)?;
        scene::capture_settled(run, &mut c, "auto-files", &expected[1])?;
        // Actual edge drop, then pull down from the maximized title.
        scene::drag(&mut c, &mut p, points.to_top[0], points.to_top[1])?;
        scene::capture_settled(run, &mut c, "auto-max", &expected[2])?;
        scene::drag(&mut c, &mut p, points.pull_down[0], points.pull_down[1])?;
        scene::capture_settled(run, &mut c, "auto-max-restored", &expected[3])?;
        // Drop on the left edge and restore the saved size a second time.
        scene::drag(&mut c, &mut p, points.to_left[0], points.to_left[1])?;
        scene::capture_settled(run, &mut c, "auto-half", &expected[4])?;
        scene::drag(&mut c, &mut p, points.pull_out[0], points.pull_out[1])?;
        scene::capture_settled(run, &mut c, "auto-half-restored", &expected[5])?;
        key(&mut c, &["ctrl", "w"])?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P11 CLIENT_EXIT",
            Duration::from_secs(15),
        )?;
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(30),
        )?;
        seal_serial(&run.root, &mut record)?;
        let text = serial(&run.root, &record)?;
        let retired = shutdown::retirement(&text, 1)?;
        let detail = json!({"image":image,"record":record,"mode":selected,"native_output":[width,height],"logical_output":[lw,lh],
            "scale_percent":scale.percent(),"hint_percent":hint,"retirement":retired,"disk":disk(&disk_path)?,"title_drag_cases":["edge-maximize-restore","edge-half-restore"],
            "owner_volume_used":false,"visible_sdl_quality":"unqualified","fps_claim":false});
        run.check("desktop-native-default", detail.clone());
        let report = json!({"id":run.id,"output_layout":run.output_layout.declaration(&run.id),"checks":run.checks(),"commands":run.commands(),"source_snapshot":run.build_inputs()});
        validate(&run.root, &report, &detail)?;
        Ok(())
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = c.execute("quit", json!({}));
    }
    result
}

fn validate_choice(
    probe: &Value,
    selection: &Value,
    stdout: Option<&str>,
) -> Result<String, String> {
    let detected = stdout.and_then(|s| session::native_display::single_mode(s.trim()));
    let selected = detected.clone().unwrap_or_else(|| "1280x720".into());
    if probe["requested"] != "auto"
        || probe["selected"] != selected
        || probe["detected_single_supported_adapter_mode"] != detected.is_some()
        || selection["requested_resolution"] != "auto"
        || selection["resolution"] != selected
    {
        return Err("automatic mode choice differs from actual probe".into());
    }
    Ok(selected)
}
fn query_succeeded(command: &Value) -> Result<bool, String> {
    let timed_out = command["timed_out"]
        .as_bool()
        .ok_or("auto query timeout flag")?;
    let limited = command["output_limit_exceeded"]
        .as_bool()
        .ok_or("auto query limit flag")?;
    let object = command.as_object().ok_or("auto query receipt")?;
    if ![
        "spawn_error",
        "cleanup_error",
        "output_budget_error",
        "exit_code",
    ]
    .iter()
    .all(|k| object.contains_key(*k))
        || !command["cleanup_error"].is_null()
        || !command["output_budget_error"].is_null()
        || command["expected_exit_code"] != 0
        || command["required"] != false
        || command["timeout_ms"] != 10000
        || command["environment_overrides"] != json!([])
        || command["watched_output_limits"] != json!([])
    {
        return Err("auto query process contract".into());
    }
    if !command["exit_code"].is_null()
        && command["exit_code"]
            .as_i64()
            .is_none_or(|n| i32::try_from(n).is_err())
    {
        return Err("auto query exit code type".into());
    }
    if !command["spawn_error"].is_null()
        && (command["spawn_error"].as_str().is_none_or(|s| s.is_empty())
            || !command["exit_code"].is_null()
            || timed_out
            || limited)
    {
        return Err("auto query impossible spawn failure".into());
    }
    let success =
        command["exit_code"] == 0 && !timed_out && !limited && command["spawn_error"].is_null();
    let outcome = if timed_out {
        "timeout"
    } else if limited {
        "output_limit"
    } else if !command["spawn_error"].is_null() {
        "tool_error"
    } else if success {
        "passed"
    } else {
        "failed"
    };
    if command["outcome"] != outcome {
        return Err("auto query outcome contradicts process result".into());
    }
    Ok(success)
}
pub(super) fn validate(root: &Path, report: &Value, detail: &Value) -> Result<(), String> {
    let checks = report["checks"].as_array().ok_or("auto checks")?;
    let probe = one(checks, "desktop-native-mode-selection")?;
    let selection = one(checks, "desktop-launch-selection")?;
    let expected = json!([
        "powershell.exe",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        session::native_display::MODE_QUERY
    ]);
    let commands: Vec<_> = report["commands"]
        .as_array()
        .ok_or("auto commands")?
        .iter()
        .filter(|c| c["argv"] == expected)
        .collect();
    if commands.len() != 1 {
        return Err("one actual automatic-mode query required".into());
    }
    let command = commands[0];
    let success = query_succeeded(command)?;
    for stream in ["stdout", "stderr"] {
        let path = paths::existing(
            root,
            command[stream]["path"]
                .as_str()
                .ok_or("auto query stream path")?,
        )?;
        if file_hash(&path)? != command[stream]["sha256"] {
            return Err("auto query stream changed".into());
        }
    }
    let path = paths::existing(
        root,
        command["stdout"]["path"]
            .as_str()
            .ok_or("auto query stream")?,
    )?;
    if file_hash(&path)? != command["stdout"]["sha256"] {
        return Err("auto query stream changed".into());
    }
    let stdout = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mode = validate_choice(probe, selection, success.then_some(stdout.as_str()))?;
    if detail["mode"] != mode || selection["requested_accelerator"] != "whpx" {
        return Err("auto mode/backend summary".into());
    }
    let record = &detail["record"];
    super::validate::validate_record(
        root,
        report,
        record,
        detail["image"].as_str().ok_or("auto image")?,
    )?;
    let args = record["argv"].as_array().ok_or("auto actual argv")?;
    if !args
        .windows(2)
        .any(|a| a[0] == "-fw_cfg" && a[1] == format!("name=opt/door/display,string={mode}"))
        || !args.windows(2).any(|a| a[0] == "-accel" && a[1] == "whpx")
        || !args
            .windows(2)
            .any(|a| a[0] == "-device" && a[1] == "ramfb")
    {
        return Err("auto actual process mode/backend".into());
    }
    let (w, h) = mode.split_once('x').ok_or("auto retained mode")?;
    let (w, h) = (
        w.parse::<u32>().map_err(|_| "auto retained width")?,
        h.parse::<u32>().map_err(|_| "auto retained height")?,
    );
    // The scale follows the recorded host hint, which reaches the guest only
    // through the actual fw_cfg argument.
    let hint = one(checks, "desktop-native-scale-hint")?["hint_percent"].as_u64();
    let scale = automatic_scale(w, h, hint)?;
    let hinted = args
        .windows(2)
        .filter(|a| {
            a[0] == "-fw_cfg"
                && a[1]
                    .as_str()
                    .is_some_and(|s| s.starts_with("name=opt/door/scale,"))
        })
        .map(|a| a[1].clone())
        .collect::<Vec<_>>();
    if hinted
        != hint
            .map(|p| json!(format!("name=opt/door/scale,string={p}")))
            .into_iter()
            .collect::<Vec<_>>()
    {
        return Err("auto scale hint argument differs from the recorded hint".into());
    }
    let points = window_points::native_default((w, h), scale)?;
    if detail["native_output"] != json!([w, h])
        || detail["logical_output"] != json!([points.logical.0, points.logical.1])
        || detail["scale_percent"] != scale.percent()
        || detail["hint_percent"] != json!(hint)
        || detail["title_drag_cases"] != json!(["edge-maximize-restore", "edge-half-restore"])
        || detail["owner_volume_used"] != false
        || detail["visible_sdl_quality"] != "unqualified"
        || detail["fps_claim"] != false
    {
        return Err("auto retained geometry or cases".into());
    }
    let empty = scene::validate_capture(root, one(checks, "desktop-auto-empty")?)?;
    let expected = expected_probes(&empty, &points)?;
    for (name, expected) in CAPTURES.iter().zip(expected) {
        let capture = one(checks, &format!("desktop-{name}"))?;
        let pixels = scene::validate_capture(root, capture)?;
        if (pixels.width, pixels.height) != (w as usize, h as usize) {
            return Err("auto capture extent changed".into());
        }
        if capture["expected_pixels"] != json!(expected) {
            return Err("auto mandatory geometry probes omitted or changed".into());
        }
    }
    let text = serial(root, record)?;
    if shutdown::retirement(&text, 1)? != detail["retirement"] {
        return Err("auto retirement changed".into());
    }
    let raw = paths::existing(root, record["disk"].as_str().ok_or("auto private disk")?)?;
    if disk(&raw)? != detail["disk"] {
        return Err("auto private persistent bytes changed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_query_result_requires_actual_consistent_process_receipt() {
        let receipt = json!({"timed_out":false,"output_limit_exceeded":false,"spawn_error":null,"cleanup_error":null,
            "output_budget_error":null,"exit_code":0,"expected_exit_code":0,"required":false,"timeout_ms":10000,
            "environment_overrides":[],"watched_output_limits":[],"outcome":"passed"});
        assert!(query_succeeded(&receipt).unwrap());
        for (field, value) in [
            ("exit_code", json!(1)),
            ("exit_code", json!("bad")),
            ("exit_code", json!(2147483648i64)),
            ("timed_out", json!(true)),
            ("spawn_error", json!("missing")),
            ("spawn_error", json!("")),
            ("spawn_error", json!(true)),
            ("cleanup_error", json!("unreaped")),
            ("output_limit_exceeded", json!(true)),
            ("required", json!(true)),
        ] {
            let mut r = receipt.clone();
            r[field] = value;
            assert!(query_succeeded(&r).is_err());
        }
        for field in [
            "exit_code",
            "spawn_error",
            "cleanup_error",
            "timed_out",
            "environment_overrides",
        ] {
            let mut r = receipt.clone();
            r.as_object_mut().unwrap().remove(field);
            assert!(query_succeeded(&r).is_err());
        }
        for (field, value, outcome) in [
            ("exit_code", json!(1), "failed"),
            ("spawn_error", json!("missing"), "tool_error"),
            ("timed_out", json!(true), "timeout"),
            ("output_limit_exceeded", json!(true), "output_limit"),
        ] {
            let mut r = receipt.clone();
            r[field] = value;
            r["outcome"] = json!(outcome);
            if field == "spawn_error" {
                r["exit_code"] = Value::Null;
            }
            assert!(!query_succeeded(&r).unwrap());
        }
        let mut impossible = receipt.clone();
        impossible["spawn_error"] = json!("missing");
        impossible["outcome"] = json!("tool_error");
        assert!(query_succeeded(&impossible).is_err());
        impossible["exit_code"] = json!("bad");
        impossible["spawn_error"] = Value::Null;
        impossible["outcome"] = json!("failed");
        assert!(query_succeeded(&impossible).is_err());
    }
    #[test]
    fn automatic_selection_cannot_be_rebound_to_another_mode_or_explicit_override() {
        let probe = json!({"requested":"auto","selected":"2560x1600","detected_single_supported_adapter_mode":true});
        let selection = json!({"requested_resolution":"auto","resolution":"2560x1600"});
        let stdout = r#"[{"width":2560,"height":1600}]"#;
        assert_eq!(
            validate_choice(&probe, &selection, Some(stdout)).unwrap(),
            "2560x1600"
        );
        for (field, value) in [
            ("selected", json!("3840x2160")),
            ("detected_single_supported_adapter_mode", json!(false)),
            ("requested", json!("2560x1600")),
        ] {
            let mut changed = probe.clone();
            changed[field] = value;
            assert!(validate_choice(&changed, &selection, Some(stdout)).is_err());
        }
        assert!(validate_choice(&probe, &selection, None).is_err());
        let mut changed = selection.clone();
        changed["requested_resolution"] = json!("2560x1600");
        assert!(validate_choice(&probe, &changed, Some(stdout)).is_err());
    }
}
