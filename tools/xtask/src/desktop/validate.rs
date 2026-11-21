//! Re-open retained guest pixels, logs, artifact pairs and actual raw-disk bytes.
use super::*;
use std::path::Path;
/// Re-open immutable retained evidence without representing it as a new boot.
pub fn revalidate(run: &mut Run, name: &str) -> Result<(), String> {
    let path = paths::existing(&run.root, name)?;
    let hash = file_hash(&path)?;
    let report = crate::image::read_json(&path)?;
    if name
        != format!(
            "reports/runs/{}/report.json",
            report["id"].as_str().ok_or("retained run id")?
        )
    {
        return Err("retained report path/id differ".into());
    }
    validate(&run.root, &report)?;
    // Even a complete evidence audit cannot promote an originally failed run.
    // Doing the read-only audit first also exposes stale validation formats while
    // preserving the failed execution envelope and every original byte.
    validate_envelope(&report)?;
    let checks = report["checks"].as_array().ok_or("retained checks")?;
    let wide: Vec<_> = checks
        .iter()
        .filter(|c| c["id"] == "desktop-wide-lifecycle")
        .collect();
    if wide.len() != 1 {
        return Err("one retained wide lifecycle required".into());
    }
    let canonical = shutdown::canonical(&serial(&run.root, &wide[0]["details"]["first"])?)?;
    if file_hash(&path)? != hash {
        return Err("original report changed during revalidation".into());
    }
    let checkpoint = json!({"schema_version":1,"kind":"retained_desktop_revalidation",
        "original_report":name,"original_report_sha256":hash,"original_run_id":report["id"],
        "guest_execution_source":report["source_snapshot"],"validation_source":run.build_inputs(),
        "guest_execution":"retained; no new guest execution in this run",
        "validated":"complete desktop validator: launch receipts, private images, sealed serials, screenshots, independent app artifacts and raw saved bytes",
        "canonical_shutdown_save":canonical});
    let checkpoint_path = run.directory.join("desktop-revalidation.json");
    write_json(&checkpoint_path, &checkpoint)?;
    run.check("retained-desktop-revalidation",json!({"checkpoint":relative(&run.root,&checkpoint_path)?,
        "sha256":file_hash(&checkpoint_path)?,"original_report":name,"original_report_sha256":hash,
        "guest_execution_source":report["source_snapshot"]["content_sha256"],
        "validation_source":run.build_inputs()["content_sha256"],"canonical_shutdown_save":canonical}));
    Ok(())
}
fn validate_envelope(report: &Value) -> Result<(), String> {
    if report["success"] != true
        || report["inputs_unchanged"] != true
        || !report["failure"].is_null()
        || !report["source_snapshot"]["content_sha256"].is_string()
        || report["source_snapshot"] != report["source_snapshot_after"]
        || report["checks"].as_array().is_none_or(|checks| {
            checks.is_empty() || checks.iter().any(|c| c["outcome"] != "passed")
        })
    {
        return Err(
            "retained report must be successful with identical complete source snapshots".into(),
        );
    }
    Ok(())
}
fn settings_coverage(layout: &Value, checks: &[Value], serial: &str) -> Result<(), String> {
    let pages = layout["pages"].as_u64().ok_or("Settings page count")?;
    if !matches!(pages, 13 | 15) {
        return Err("Settings native coverage missing".into());
    }
    let marker = |page| {
        serial.lines().any(|l| {
            l.starts_with("DOOR_USER pid=")
                && l.ends_with(&format!(" DOOR_P12 SETTINGS PAGE id={page}"))
        })
    };
    let modern = pages == 15
        || marker(13)
        || marker(14)
        || checks.iter().any(|c| {
            matches!(
                c["id"].as_str(),
                Some(
                    "desktop-settings-display"
                        | "desktop-settings-display-narrow"
                        | "desktop-settings-installed-apps"
                )
            )
        });
    if modern {
        if pages != 15 {
            return Err("modern Settings coverage was downgraded".into());
        }
        for name in ["about", "display", "display-narrow", "installed-apps"] {
            if checks
                .iter()
                .filter(|c| {
                    c["id"] == format!("desktop-settings-{name}") && c["outcome"] == "passed"
                })
                .count()
                != 1
            {
                return Err("Settings native detail capture missing or duplicated".into());
            }
        }
        if !(12..=14).all(marker) {
            return Err("Settings actual detail-page navigation missing".into());
        }
    }
    Ok(())
}
fn maximized_readiness(detail: &Value, capture: &Value, current: bool) -> Result<bool, String> {
    let settled = detail["maximized_capture_protocol"] == 2;
    if current && !settled {
        return Err("current maximized capture readiness missing".into());
    }
    if settled
        && capture["identical_whole_frame_samples"]
            .as_u64()
            .is_none_or(|n| n < 4)
    {
        return Err("maximized frame did not settle".into());
    }
    Ok(settled)
}
pub fn validate(root: &Path, report: &Value) -> Result<(), String> {
    let layout = crate::layout::Layout::from_report(report)?;
    let checks = report["checks"].as_array().ok_or("desktop checks")?;
    let one = |id: &str| -> Result<&Value, String> {
        let matches: Vec<_> = checks
            .iter()
            .filter(|v| v["id"] == id && v["outcome"] == "passed")
            .collect();
        if matches.len() != 1 {
            return Err(format!("one passing {id} required"));
        }
        Ok(&matches[0]["details"])
    };
    if report["source_snapshot"]["files"]
        .as_array()
        .is_some_and(|files| {
            files
                .iter()
                .any(|f| f["path"] == "tools/xtask/src/desktop/native_default.rs")
        })
    {
        super::native_default::validate(root, report, one("desktop-native-default")?)?;
    }
    let suite = one("native-desktop-suite")?;
    let recovery = one("desktop-recovery-suite")?;
    if suite["outcome"] != "passed"
        || recovery["outcome"] != "passed"
        || suite["boot_test_token"] != false
        || suite["first"]["uuid"] == suite["second"]["uuid"]
        || suite["first"]["disk"] != suite["second"]["disk"]
        || suite["disk"] != suite["first"]["disk"]
    {
        return Err("desktop boot/workflow identity".into());
    }
    for (record, image) in [
        (&suite["first"], &suite["image"]),
        (&suite["second"], &suite["image"]),
        (&recovery["record"], &recovery["image"]),
    ] {
        validate_record(
            root,
            report,
            record,
            image.as_str().ok_or("desktop source image")?,
        )?;
    }
    for id in ["desktop-first-render", "desktop-notes-workflow"] {
        let detail = one(id)?;
        if file_hash(&paths::existing(
            root,
            detail["screenshot"].as_str().ok_or("desktop screenshot")?,
        )?)? != detail["sha256"]
        {
            return Err("desktop workflow screenshot changed".into());
        }
    }
    if file_hash(&paths::existing(
        root,
        recovery["screenshot"]
            .as_str()
            .ok_or("recovery screenshot")?,
    )?)? != recovery["screenshot_sha256"]
    {
        return Err("recovery screenshot changed".into());
    }
    let disk = paths::existing(root, suite["disk"].as_str().ok_or("desktop disk")?)?;
    require_folders(&suite["saved"])?;
    require_folders(&suite["reopened"])?;
    if audit(&disk, b"Persistent desktop note\n reopened\n")? != suite["reopened"] {
        return Err("desktop persisted bytes changed".into());
    }
    let disk = paths::existing(
        root,
        recovery["record"]["disk"].as_str().ok_or("recovery disk")?,
    )?;
    if audit(&disk, b"Before restart\nRecovered desktop note\n")? != recovery["audit"] {
        return Err("recovery persisted bytes changed".into());
    }
    let text = serial(root, &recovery["record"])?;
    recovery::validate_binding_summary(&text, checks)?;
    for needle in [
        "DOOR_P11 ADVERSARY PASS",
        "reuse=32",
        "DOOR_P11 RECOVERY INJECT",
        "DOOR_P11 HUNG_CLOSE PASS",
        "restart=1",
    ] {
        if !text.contains(needle) {
            return Err(format!("missing actual guest result {needle}"));
        }
    }
    if !reconnected_both(text.split("DOOR_P11 HUNG_CLOSE PASS").next().unwrap_or("")) {
        return Err("both clients must reconnect independently".into());
    }
    scene::validate(root, checks)?;
    // Preserve historical P11 reports, but require the whole newer output suite
    // whenever this batch declares its wider lifecycle gate.
    if checks.iter().any(|c| c["id"] == "desktop-wide-lifecycle") {
        let wide = one("desktop-wide-lifecycle")?;
        validate_wide_identity(wide)?;
        // Earlier retained reports predate explicit document choice. Protocol2
        // additionally binds the live shutdown Save to a retained durable image.
        let wide_serial = serial(root, &wide["first"])?;
        if shutdown::required(wide, checks, &wide_serial) {
            let saved = one("desktop-shutdown-save-dialog")?;
            let canonical = shutdown::canonical(&wide_serial)?;
            shutdown::validate_summary(saved, &canonical)?;
            let snapshot = paths::existing(
                root,
                saved["snapshot"].as_str().ok_or("shutdown save snapshot")?,
            )?;
            if file_hash(&snapshot)? != saved["sha256"]
                || audit(&snapshot, b"Saved during shutdown\n")? != saved["saved"]
            {
                return Err("shutdown dialog saved bytes changed".into());
            }
        }
        settings_coverage(one("desktop-settings-layouts")?, checks, &wide_serial)?;
        if one("desktop-settings-themes")?["themes"] != 6 {
            return Err("Settings native coverage missing".into());
        }
        for index in 0..12 {
            one(&format!("desktop-settings-page-{index}"))?;
        }
        for index in 0..6 {
            one(&format!("desktop-settings-theme-{index}"))?;
        }
        for record in [&wide["first"], &wide["second"]] {
            validate_record(
                root,
                report,
                record,
                wide["image"].as_str().ok_or("wide image")?,
            )?;
        }
        let disk = paths::existing(root, wide["disk"].as_str().ok_or("wide disk")?)?;
        if checks
            .iter()
            .any(|c| c["id"] == "desktop-advanced-settings")
        {
            if super::administration::audit(&disk)? != *one("desktop-advanced-settings-durable")? {
                return Err("advanced Settings persisted value changed".into());
            }
            for id in 17..=32 {
                one(&format!("desktop-admin-page-{id}"))?;
            }
            for suffix in ["saved", "invalid-draft"] {
                one(&format!("desktop-admin-registry-{suffix}"))?;
            }
        }
        if file_hash(&disk)? != wide["disk_sha256"] {
            return Err("wide persisted disk changed".into());
        }
        audit(&disk, b"Saved during shutdown\n")?;
        if lifecycle::appearance_on_disk(&disk)? != 0 {
            return Err("wide final light theme changed".into());
        }
        let native_density = checks.iter().any(|c| {
            c["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("desktop-native-density-"))
        });
        let modes = if native_density {
            &[(1920, 1080), (2560, 1600)][..]
        } else {
            &[(1920, 1080), (2560, 1600), (3840, 2160)][..]
        };
        for &(width, height) in modes {
            let mode = format!("{width}x{height}");
            let detail = one(&format!("desktop-resolution-{mode}"))?;
            validate_mode(detail, width, height)?;
            validate_record(
                root,
                report,
                &detail["record"],
                suite["image"].as_str().ok_or("resolution image")?,
            )?;
            let mode_serial = serial(root, &detail["record"])?;
            let current_mode = detail["document_close_protocol"] == 2
                || mode_serial.contains("DOOR_P12 NOTES PROMPT intent=Close");
            if current_mode {
                shutdown::canonical_for(&mode_serial, 9)?;
            }
            let disk = paths::existing(
                root,
                detail["record"]["disk"].as_str().ok_or("resolution disk")?,
            )?;
            if audit(&disk, b"High resolution note\n")? != detail["audit"] {
                return Err("native resolution saved bytes changed".into());
            }
            for index in 0..4 {
                let suffix = if index == 0 {
                    "empty".into()
                } else {
                    format!("max-{}", index - 1)
                };
                let capture = one(&format!("desktop-{mode}-{suffix}"))?;
                let pixels = scene::Pixels::read(&paths::existing(
                    root,
                    capture["ppm"].as_str().ok_or("mode ppm")?,
                )?)?;
                if (pixels.width, pixels.height) != (width as usize, height as usize) {
                    return Err("retained native scanout extent differs".into());
                }
                if index > 0 {
                    let color = if index == 3 {
                        super::resolution::SETTINGS_BACKGROUND
                    } else {
                        [255, 255, 255]
                    };
                    let settled = maximized_readiness(detail, capture, current_mode)?;
                    let y = if settled && index != 3 {
                        height / 2
                    } else {
                        height - 100
                    };
                    if pixels.pixel(width as usize - 10, y as usize)? != color {
                        return Err("retained maximized client pixels differ".into());
                    }
                } else if width == 3840
                    && (pixels.pixel(0, 0)? != [133, 174, 202]
                        || pixels.pixel(3839, 0)? != [126, 170, 199])
                {
                    return Err("retained 4K wallpaper pixels differ".into());
                }
            }
        }
        if native_density {
            for (width, height, scale) in [(2560, 1440, 2), (3840, 2160, 3)] {
                let detail = one(&format!("desktop-native-density-{width}x{height}"))?;
                validate_mode(detail, width, height)?;
                if detail["logical_output"] != json!([1280, 720])
                    || detail["scale"] != scale
                    || detail["dpi"] != 96 * scale
                    || detail["source_rasterization"] != true
                    || detail["host_enlargement"] != false
                {
                    return Err("native density source-rasterization identity differs".into());
                }
                validate_record(
                    root,
                    report,
                    &detail["record"],
                    suite["image"].as_str().ok_or("density image")?,
                )?;
                let mode_serial = serial(root, &detail["record"])?;
                if mode_serial.contains("DOOR_P12 NOTES PROMPT intent=Close") {
                    shutdown::canonical_for(&mode_serial, 8)?;
                }
                motion::validate_saved(&mode_serial, &detail["motion"])?;
                let disk = paths::existing(
                    root,
                    detail["record"]["disk"].as_str().ok_or("density disk")?,
                )?;
                if audit(&disk, b"Native density note\n")? != detail["audit"] {
                    return Err("native density saved bytes changed".into());
                }
            }
        }
        for check in checks.iter().filter(|c| c["details"]["ppm"].is_string()) {
            let detail = &check["details"];
            for (file, hash) in [("ppm", "ppm_sha256"), ("screenshot", "sha256")] {
                if file_hash(&paths::existing(
                    root,
                    detail[file].as_str().ok_or("capture path")?,
                )?)? != detail[hash]
                {
                    return Err("retained native output capture changed".into());
                }
            }
        }
    }
    let build_path = paths::existing(
        root,
        &format!(
            "reports/runs/{}/desktop-build.json",
            report["id"].as_str().ok_or("desktop run id")?
        ),
    )?;
    let build = crate::image::read_json(&build_path)?;
    let owner = report["id"].as_str().ok_or("desktop layout owner")?;
    if layout == crate::layout::Layout::Legacy
        && build["image"]
            .as_str()
            .is_some_and(|path| path.starts_with("reports/runs/"))
    {
        return Err("retained desktop image requires a v2 declaration".into());
    }
    if (layout == crate::layout::Layout::RetainedDesktop || !build["output_layout"].is_null())
        && build["output_layout"] != layout.declaration(owner)
    {
        return Err("desktop build/report output layouts differ".into());
    }
    if layout == crate::layout::Layout::RetainedDesktop {
        for (key, suffix) in [("image", "desktop"), ("test_image", "desktop-test")] {
            if build[key]
                != layout.path(owner, &format!("out/images/{owner}-{suffix}/image.json"))?
            {
                return Err("desktop image outside declared generated tree".into());
            }
        }
        let prefix = layout.path(owner, &format!("out/builds/{owner}"))? + "/";
        for artifact in build["artifacts"].as_array().ok_or("desktop artifacts")? {
            if !artifact["path"]
                .as_str()
                .is_some_and(|path| path.starts_with(&prefix))
            {
                return Err("desktop artifact outside declared generated tree".into());
            }
        }
    }
    if !build["development_relink"].is_null()
        || build["source_snapshot"] != report["source_snapshot"]
        || build["image"] != suite["image"]
        || build["test_image"] != recovery["image"]
    {
        return Err("desktop qualification used stale/development artifacts".into());
    }
    for (path, test) in [(&build["image"], false), (&build["test_image"], true)] {
        let (base, _) = crate::image::validate_staged(root, path.as_str().ok_or("desktop image")?)?;
        let bytes = fs::read(base.join("esp/Door/initfs.tar")).map_err(|e| e.to_string())?;
        let archive =
            door_initfs::Archive::parse(&bytes).map_err(|e| format!("desktop initfs: {e:?}"))?;
        if archive.file("system/window-tests.conf").is_some() != test
            || archive.file("bin/native-workload.elf").is_some() != test
            || archive.file("system/desktop.conf").is_none()
        {
            return Err("desktop production/test separation".into());
        }
        if archive
            .file(crate::app_assets::DESTINATION)
            .is_none_or(|pack| crate::app_assets::validate(pack).is_err())
        {
            return Err("desktop application asset pack".into());
        }
    }
    // The Captions diagnostics flag is in neither the shipping nor the
    // adversarial image; the desktop-diagnostics image is the shipping
    // initfs plus exactly that one file.
    let initfs_of = |path: &Value| -> Result<Vec<u8>, String> {
        let (base, _) = crate::image::validate_staged(root, path.as_str().ok_or("desktop image")?)?;
        fs::read(base.join("esp/Door/initfs.tar")).map_err(|e| e.to_string())
    };
    let (flag_path, _, flag) = crate::session::ASSIST_DIAGNOSTICS;
    for path in [&build["image"], &build["test_image"]] {
        let bytes = initfs_of(path)?;
        let archive =
            door_initfs::Archive::parse(&bytes).map_err(|e| format!("desktop initfs: {e:?}"))?;
        if archive.file(flag_path).is_some() {
            return Err("Captions diagnostics flag outside the diagnostics image".into());
        }
    }
    if !build["images"].is_null() {
        if layout == crate::layout::Layout::RetainedDesktop
            && build["diagnostics_image"]
                != layout.path(
                    owner,
                    &format!("out/images/{owner}-desktop-diagnostics/image.json"),
                )?
        {
            return Err("desktop image outside declared generated tree".into());
        }
        let production = initfs_of(&build["image"])?;
        let diagnostics = initfs_of(&build["diagnostics_image"])?;
        if diagnostics != crate::initfs::with_file(&production, flag_path, flag)? {
            return Err(
                "desktop-diagnostics image is not the shipping initfs plus its flag".into(),
            );
        }
    }
    let rebuilds = one("desktop-application-rebuilds")?
        .as_array()
        .ok_or("desktop rebuilds")?;
    for name in [
        "door-shell",
        "door-tools",
        "door-window",
        "door-files",
        "door-notes",
        "door-settings",
        "door-terminal",
        "door-search",
        "door-desktop-shell",
        "door-inbox",
        "door-network",
        "door-audio",
        "door-ime",
        "door-gallery",
        "door-assist",
        crate::app_assets::NAME,
        "tinybasic",
    ] {
        let pair = rebuilds
            .iter()
            .find(|v| v["name"] == name)
            .ok_or("desktop artifact pair")?;
        let paths = pair["paths"].as_array().ok_or("desktop pair paths")?;
        if paths.len() != 2 || paths[0] == paths[1] {
            return Err("independent desktop build paths".into());
        }
        for path in paths {
            if layout == crate::layout::Layout::RetainedDesktop {
                let prefix = layout.path(owner, &format!("out/builds/{owner}"))? + "/";
                if !path.as_str().is_some_and(|path| path.starts_with(&prefix)) {
                    return Err("desktop repeated artifact outside declared generated tree".into());
                }
            }
            if file_hash(&crate::paths::existing(
                root,
                path.as_str().ok_or("desktop pair path")?,
            )?)? != pair["sha256"]
            {
                return Err("desktop artifact changed".into());
            }
        }
    }
    Ok(())
}
fn validate_wide_identity(wide: &Value) -> Result<(), String> {
    if !wide["disk"].is_string()
        || !wide["first"]["uuid"].is_string()
        || !wide["second"]["uuid"].is_string()
        || wide["first"]["uuid"] == wide["second"]["uuid"]
        || wide["first"]["disk"] != wide["disk"]
        || wide["second"]["disk"] != wide["disk"]
    {
        return Err("wide separate boot/disk identity differs".into());
    }
    Ok(())
}
fn validate_mode(detail: &Value, width: u32, height: u32) -> Result<(), String> {
    let expected = format!("name=opt/door/display,string={width}x{height}");
    let argv = detail["record"]["argv"].as_array().ok_or("mode argv")?;
    validate_display_args(argv)?;
    if detail["native_output"] != json!([width, height])
        || !argv
            .windows(2)
            .any(|w| w[0] == "-fw_cfg" && w[1] == expected)
    {
        return Err("native output mode was rebound".into());
    }
    Ok(())
}
pub(super) fn validate_record(
    root: &Path,
    report: &Value,
    record: &Value,
    image: &str,
) -> Result<(), String> {
    validate_record_layout(report, record)?;
    if record["image"] != image {
        return Err("actual desktop launch image differs from suite".into());
    }
    let argv = record["argv"].as_array().ok_or("desktop QEMU argv")?;
    validate_launch_disk(root, record)?;
    let serial_path = paths::existing(root, record["serial"].as_str().ok_or("desktop serial")?)?;
    let private = serial_path.parent().ok_or("desktop private directory")?;
    let (_, staged) = crate::image::validate_staged(root, image)?;
    let (_, copied) =
        crate::image::validate_staged(root, &relative(root, &private.join("image.json"))?)?;
    if staged != copied {
        return Err("actual copied desktop ESP differs from qualified image".into());
    }
    let profile = crate::image::read_json(&root.join("profiles/hardware/qemu-services-x64.json"))?;
    let (_, code, _) = crate::emulator::acquired_inputs(&profile)?;
    let escaped = |p: &Path| {
        p.to_string_lossy()
            .trim_start_matches("\\\\?\\")
            .replace('\\', "/")
            .replace(',', ",,")
    };
    let has = |flag: &str, value: &str| {
        argv.windows(2)
            .filter(|w| w[0] == flag && w[1] == value)
            .count()
            == 1
    };
    for value in [
        format!(
            "if=pflash,format=raw,unit=0,readonly=on,file={}",
            escaped(&code)
        ),
        format!(
            "if=pflash,format=raw,unit=1,file={}",
            escaped(&private.join("vars.fd"))
        ),
        format!(
            "if=none,id=door-esp,format=raw,readonly=on,file=fat:{}",
            escaped(&private.join("esp"))
        ),
    ] {
        if !has("-drive", &value) {
            return Err("actual desktop firmware/ESP/data-disk command differs".into());
        }
    }
    if !has(
        "-serial",
        &format!("file:{}", serial_path.to_string_lossy()),
    ) || !has("-uuid", record["uuid"].as_str().ok_or("desktop uuid")?)
    {
        return Err("actual desktop serial/uuid command differs".into());
    }
    let mut expected = vec![profile["qemu"]["path"].clone()];
    expected.extend(argv.iter().cloned());
    if !report["commands"]
        .as_array()
        .ok_or("desktop launch commands")?
        .iter()
        .any(|command| {
            command["argv"] == json!(expected)
                && command["pid"] == record["pid"]
                && command["outcome"] == "running"
                && command["spawn_error"].is_null()
                && command["cleanup_error"].is_null()
                && command["controller_error"].is_null()
                && command["timed_out"] == false
                && viewer_environment_valid(
                    report,
                    record,
                    command,
                    profile["qemu"]["path"]
                        .as_str()
                        .is_some_and(|p| p.to_ascii_lowercase().ends_with(".exe")),
                )
        })
    {
        return Err("actual desktop QEMU command receipt missing".into());
    }
    validate_display_args(argv)?;
    if let Some(retained) = record["serial_retained"].as_str()
        && file_hash(&paths::existing(root, retained)?)? != record["serial_sha256"]
    {
        return Err("retained desktop serial changed".into());
    }
    if file_hash(&paths::existing(
        root,
        record["serial"].as_str().ok_or("desktop serial")?,
    )?)? != record["serial_sha256"]
    {
        return Err("desktop serial changed".into());
    }
    let text = serial(root, record)?;
    if !text.contains("CLEANUP PASS")
        || !text.contains("leaked=0")
        || !text.contains("reclaimed=8")
        || ["KERNEL_PANIC", "APP FAILED", "INIT FAIL", "ADVERSARY FAIL"]
            .iter()
            .any(|s| text.contains(s))
    {
        return Err("desktop guest failure or missing cleanup".into());
    }
    Ok(())
}

fn viewer_environment_valid(
    report: &Value,
    record: &Value,
    command: &Value,
    windows: bool,
) -> bool {
    let current = report["source_snapshot"]["files"]
        .as_array()
        .is_some_and(|files| {
            files
                .iter()
                .any(|f| f["path"] == "tools/xtask/src/session/native_display.rs")
        });
    if !current {
        return record["viewer_environment"].is_null()
            || command["environment_overrides"] == record["viewer_environment"];
    }
    let Some(visible) = record["visible"].as_bool() else {
        return false;
    };
    let Some(argv) = record["argv"].as_array() else {
        return false;
    };
    let displays: Vec<_> = argv
        .windows(2)
        .filter(|a| a[0] == "-display")
        .map(|a| &a[1])
        .collect();
    if displays != vec![&json!(if visible { "sdl" } else { "none" })] {
        return false;
    }
    let expected = if visible && windows {
        json!(session::NATIVE_PIXEL_ENV)
    } else {
        json!([])
    };
    record["viewer_environment"] == expected && command["environment_overrides"] == expected
}

fn validate_record_layout(report: &Value, record: &Value) -> Result<(), String> {
    let layout = crate::layout::Layout::from_report(report)?;
    let owner = report["id"].as_str().ok_or("desktop owner")?;
    if layout == crate::layout::Layout::Legacy
        && record["serial"]
            .as_str()
            .is_some_and(|path| path.starts_with("reports/runs/"))
    {
        return Err("retained desktop launch requires a v2 declaration".into());
    }
    if layout == crate::layout::Layout::RetainedDesktop || !record["output_layout"].is_null() {
        if record["output_layout"] != layout.declaration(owner) || record["layout_owner"] != owner {
            return Err("desktop launch/report output layouts differ".into());
        }
        let instance = record["private_instance"]
            .as_str()
            .ok_or("desktop private instance")?;
        let stamp = instance
            .strip_prefix(&format!("{owner}-session-"))
            .ok_or("desktop instance owner")?;
        if !stamp
            .parse::<u128>()
            .is_ok_and(|n| n != 0 && n.to_string() == stamp)
        {
            return Err("noncanonical desktop instance stamp".into());
        }
        let directory = layout.path(owner, &format!("out/qemu/{instance}"))?;
        if record["serial"] != format!("{directory}/serial.log") {
            return Err("desktop private serial outside declared generated tree".into());
        }
    }
    Ok(())
}

fn validate_display_args(argv: &[Value]) -> Result<(), String> {
    let (mut displays, mut scales) = (0, 0);
    for (i, arg) in argv.iter().enumerate() {
        if arg.as_str().is_some_and(|s| s.contains("isa-debug-exit")) {
            return Err("desktop used a diagnostic exit".into());
        }
        if arg == "-fw_cfg" {
            let value = argv.get(i + 1).and_then(Value::as_str).unwrap_or("");
            // The host display scale hint of an automatic launch (W8): one
            // canonical offered percentage, beside the display mode.
            if let Some(percent) = value.strip_prefix("name=opt/door/scale,string=") {
                scales += 1;
                let offered = percent
                    .parse::<u32>()
                    .ok()
                    .filter(|p| p.to_string() == percent)
                    .and_then(door_window::graphics::Scale::from_percent)
                    .is_some();
                if !offered || scales > 1 {
                    return Err("desktop used an unbounded/diagnostic firmware input".into());
                }
                continue;
            }
            displays += 1;
            if !matches!(
                value,
                "name=opt/door/display,string=1024x768"
                    | "name=opt/door/display,string=1280x720"
                    | "name=opt/door/display,string=1920x1080"
                    | "name=opt/door/display,string=1920x1200"
                    | "name=opt/door/display,string=2560x1440"
                    | "name=opt/door/display,string=2560x1600"
                    | "name=opt/door/display,string=3840x2160"
            ) || displays > 1
            {
                return Err("desktop used an unbounded/diagnostic firmware input".into());
            }
        }
    }
    Ok(())
}

fn validate_launch_disk(root: &Path, record: &Value) -> Result<(), String> {
    let argv = record["argv"].as_array().ok_or("desktop QEMU argv")?;
    let disk = paths::existing(root, record["disk"].as_str().ok_or("desktop launch disk")?)?;
    let escaped = disk
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/")
        .replace(',', ",,");
    let expected = format!("if=none,id=door-data,format=raw,file={escaped}");
    let drives: Vec<_> = argv
        .windows(2)
        .filter(|w| {
            w[0] == "-drive"
                && w[1]
                    .as_str()
                    .is_some_and(|value| value.split(',').any(|part| part == "id=door-data"))
        })
        .collect();
    if drives.len() != 1 || drives[0][1] != expected {
        return Err("audited desktop disk differs from actual QEMU data drive".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_viewer_receipts_cannot_strip_or_rebind_native_pixel_environment() {
        let report = json!({"source_snapshot":{"files":[{"path":"tools/xtask/src/session/native_display.rs"}]}});
        for (visible, windows) in [(true, true), (false, true), (true, false)] {
            let env = if visible && windows {
                json!(session::NATIVE_PIXEL_ENV)
            } else {
                json!([])
            };
            let record = json!({"visible":visible,"argv":["-display",if visible {"sdl"} else {"none"}],"viewer_environment":env});
            let command = json!({"environment_overrides":env});
            assert!(viewer_environment_valid(
                &report, &record, &command, windows
            ));
            for bad in [
                Value::Null,
                json!([["SDL_WINDOWS_DPI_AWARENESS", "system"]]),
            ] {
                let mut r = record.clone();
                let mut c = command.clone();
                r["viewer_environment"] = bad.clone();
                c["environment_overrides"] = bad;
                assert!(!viewer_environment_valid(&report, &r, &c, windows));
            }
            let mut r = record.clone();
            r["visible"] = Value::Null;
            assert!(!viewer_environment_valid(&report, &r, &command, windows));
            r = record.clone();
            r["argv"] = json!(["-display", "sdl", "-display", "none"]);
            assert!(!viewer_environment_valid(&report, &r, &command, windows));
        }
        assert!(viewer_environment_valid(
            &json!({}),
            &json!({}),
            &json!({}),
            true
        ));
    }
    #[test]
    fn retained_launch_requires_exact_layout_owner_and_instance() {
        let owner = "host-123-456";
        let report = json!({"id":owner,"output_layout":crate::layout::Layout::RetainedDesktop.declaration(owner)});
        let record = json!({"output_layout":report["output_layout"],"layout_owner":owner,
            "private_instance":"host-123-456-session-789",
            "serial":"reports/runs/host-123-456/generated/qemu/session-789/serial.log"});
        assert!(validate_record_layout(&report, &record).is_ok());
        for (key, value) in [
            ("output_layout", Value::Null),
            ("layout_owner", json!("host-7-8")),
            ("private_instance", json!("host-123-456-session-0789")),
            (
                "serial",
                json!("out/qemu/host-123-456-session-789/serial.log"),
            ),
        ] {
            let mut bad = record.clone();
            bad[key] = value;
            assert!(validate_record_layout(&report, &bad).is_err(), "{key}");
        }
        let legacy = json!({"id":owner});
        assert!(validate_record_layout(&legacy, &record).is_err());
        assert!(validate_record_layout(&legacy, &json!({})).is_ok());
    }
    #[test]
    fn serial_attested_current_mode_cannot_strip_readiness_metadata() {
        let detail = json!({"document_close_protocol":2,"maximized_capture_protocol":2});
        let capture = json!({"identical_whole_frame_samples":4});
        assert!(maximized_readiness(&detail, &capture, true).unwrap());
        // Current is also derived from retained Close prompt serial, independent
        // of both removable protocol scalars in the report summary.
        assert!(maximized_readiness(&json!({}), &capture, true).is_err());
        assert!(maximized_readiness(&detail, &json!({}), true).is_err());
        assert!(
            maximized_readiness(&detail, &json!({"identical_whole_frame_samples":3}), true)
                .is_err()
        );
        assert!(!maximized_readiness(&json!({}), &json!({}), false).unwrap());
    }
    #[test]
    fn modern_settings_evidence_cannot_downgrade_or_drop_detail_pages() {
        let layout = json!({"pages":15});
        let checks: Vec<_> = ["about", "display", "display-narrow", "installed-apps"]
            .iter()
            .map(|name| json!({"id":format!("desktop-settings-{name}"),"outcome":"passed"}))
            .collect();
        let serial = (12..=14)
            .map(|page| format!("DOOR_USER pid=8 DOOR_P12 SETTINGS PAGE id={page}\n"))
            .collect::<String>();
        settings_coverage(&layout, &checks, &serial).unwrap();
        assert!(settings_coverage(&json!({"pages":13}), &checks, &serial).is_err());
        assert!(settings_coverage(&json!({"pages":13}), &[], &serial).is_err());
        for i in 0..checks.len() {
            let mut missing = checks.clone();
            missing.remove(i);
            assert!(settings_coverage(&layout, &missing, &serial).is_err());
            let mut duplicate = checks.clone();
            duplicate.push(checks[i].clone());
            assert!(settings_coverage(&layout, &duplicate, &serial).is_err());
        }
        for page in 12..=14 {
            assert!(
                settings_coverage(
                    &layout,
                    &checks,
                    &serial.replace(
                        &format!("DOOR_USER pid=8 DOOR_P12 SETTINGS PAGE id={page}\n"),
                        ""
                    )
                )
                .is_err()
            );
        }
        settings_coverage(&json!({"pages":13}), &[], "genuinely historical").unwrap();
    }
    #[test]
    fn retained_revalidation_rejects_failed_or_mutated_execution() {
        let good = json!({"success":true,"inputs_unchanged":true,"failure":null,
            "source_snapshot":{"content_sha256":"abc","files":[{"path":"a","sha256":"one"}]},
            "source_snapshot_after":{"content_sha256":"abc","files":[{"path":"a","sha256":"one"}]},
            "checks":[{"outcome":"passed"}]});
        validate_envelope(&good).unwrap();
        for (key, value) in [
            ("success", json!(false)),
            ("inputs_unchanged", json!(false)),
            ("failure", json!("failed cleanup")),
            ("checks", json!([])),
            ("checks", json!([{"outcome":"failed"}])),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(validate_envelope(&bad).is_err());
        }
        let mut bad = good;
        bad["source_snapshot_after"]["files"][0]["sha256"] = json!("two");
        assert!(validate_envelope(&bad).is_err());
    }
    #[test]
    fn retained_wide_boots_and_native_modes_cannot_be_rebound() {
        let mut wide = json!({"disk":"one.raw","first":{"disk":"one.raw","uuid":"one"},"second":{"disk":"one.raw","uuid":"two"}});
        assert!(validate_wide_identity(&wide).is_ok());
        wide["second"]["uuid"] = json!("one");
        assert!(validate_wide_identity(&wide).is_err());
        wide["second"]["uuid"] = json!("two");
        wide["disk"] = json!("unrelated.raw");
        assert!(validate_wide_identity(&wide).is_err());
        let detail = json!({"native_output":[1920,1080],"record":{"argv":["-fw_cfg","name=opt/door/display,string=1920x1080"]}});
        assert!(validate_mode(&detail, 1920, 1080).is_ok());
        assert!(validate_mode(&detail, 3840, 2160).is_err());
    }
    #[test]
    fn only_one_bounded_display_firmware_input_is_ordinary() {
        let args = |v: &[&str]| v.iter().map(|s| json!(s)).collect::<Vec<_>>();
        assert!(validate_display_args(&args(&[])).is_ok());
        assert!(
            validate_display_args(&args(&[
                "-fw_cfg",
                "name=opt/door/display,string=3840x2160"
            ]))
            .is_ok()
        );
        for value in [
            "name=opt/door/test,string=1",
            "name=opt/door/display,string=9999x9999",
            "",
        ] {
            assert!(validate_display_args(&args(&["-fw_cfg", value])).is_err());
        }
        assert!(
            validate_display_args(&args(&[
                "-fw_cfg",
                "name=opt/door/display,string=1280x720",
                "-fw_cfg",
                "name=opt/door/display,string=1280x720"
            ]))
            .is_err()
        );
        // An automatic launch adds one canonical offered scale hint (W8).
        let hinted = |hint: &str| {
            validate_display_args(&args(&[
                "-fw_cfg",
                "name=opt/door/display,string=2560x1600",
                "-fw_cfg",
                hint,
            ]))
        };
        assert!(hinted("name=opt/door/scale,string=150").is_ok());
        for bad in [
            "name=opt/door/scale,string=130",
            "name=opt/door/scale,string=0150",
            "name=opt/door/scale,string=",
        ] {
            assert!(hinted(bad).is_err(), "{bad}");
        }
        assert!(
            validate_display_args(&args(&[
                "-fw_cfg",
                "name=opt/door/scale,string=150",
                "-fw_cfg",
                "name=opt/door/scale,string=150"
            ]))
            .is_err()
        );
        assert!(validate_display_args(&args(&["-device", "isa-debug-exit"])).is_err());
    }
    #[test]
    fn a_valid_disk_audit_cannot_be_rebound_to_another_attached_disk() {
        let d = crate::test_support::Workspace::new();
        fs::write(d.root.join("audited.raw"), b"audited bytes").unwrap();
        fs::write(d.root.join("other.raw"), b"different guest data").unwrap();
        let path = d.root.join("audited.raw").canonicalize().unwrap();
        let path = path
            .to_string_lossy()
            .trim_start_matches("\\\\?\\")
            .replace('\\', "/")
            .replace(',', ",,");
        let drive = format!("if=none,id=door-data,format=raw,file={path}");
        let mut record = json!({"disk":"audited.raw", "argv":["-drive",drive]});
        assert!(validate_launch_disk(&d.root, &record).is_ok());
        record["disk"] = json!("other.raw");
        assert!(validate_launch_disk(&d.root, &record).is_err());
        record["disk"] = json!("audited.raw");
        record["argv"] = json!(["-drive", drive, "-drive", drive]);
        assert!(validate_launch_disk(&d.root, &record).is_err());
        record["argv"] = json!(["-drive", "if=none,id=door-esp,format=raw,file=audited.raw"]);
        assert!(validate_launch_disk(&d.root, &record).is_err());
    }
}
