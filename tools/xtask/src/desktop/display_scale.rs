//! DISPLAY-01 (`cargo xtask test ui --suite display-01`): live display scale
//! changes through Settings > System > Display on a 2560x1600 output
//! (100..=200% offered, 200% recommended), a contrast theme switched on and
//! off with Left Alt+Left Shift+Print Screen, Accessibility > Text size, and
//! the stored scale bound again on a second boot. Evidence is guest serial
//! from the window server (`DISPLAY_SCALE ... focus=` logical bounds of the
//! focused window after each rescale), every workspace client
//! (`CLIENT_SCALE`, i.e. SCALE_CHANGED delivered and applied), Settings and
//! Init, checked against host arithmetic of the protocol's logical extent.
//! Screenshots are retained as artifacts only; they qualify nothing here.
//! Key paths are pinned by the host test `acceptance_suite_settings_key_paths`.
use super::*;

const DISPLAY_PAGE: usize = 13;
/// Tab presses from the Display page to its Scale combo: Brightness, Night
/// light (card and switch), HDR, then Scale (host test `acceptance_suite_settings_key_paths`
/// in desktop/workspace/tests/settings_reference_painter.rs).
pub(crate) const SCALE_TABS: usize = 5;
const TEXT_SIZE_PAGE: usize = 47;
const OUTPUT: (u32, u32) = (2560, 1600);
/// Offered on 2560x1600 (225% has no exported atlas and is not offered).
const OFFERED: [u32; 5] = [100, 125, 150, 175, 200];
const RECOMMENDED: u32 = 200;
/// Scale left stored for the second boot.
const PERSISTED: u32 = 150;
const CONTRAST_ON: u32 = 1 << 24;
const TEXT_SHIFT: u32 = 25;

/// `door_service_protocol::window::logical_extent`, recomputed host-side.
fn logical_extent(physical: u32, percent: u32) -> u32 {
    ((u64::from(physical) * 100 + 50) / u64::from(percent)) as u32
}
fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|w| w.strip_prefix(name))
}
/// The first window-server rescale record after `offset` (kernel prefix
/// `DOOR_USER pid=N ` included in the returned line).
fn rescale_after(text: &str, offset: usize) -> Option<&str> {
    text.get(offset..)?
        .lines()
        .find(|l| l.contains("DOOR_P12 DISPLAY_SCALE preference="))
}
fn wait_rescale(run: &Run, record: &Value, offset: usize) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let text = serial(&run.root, record)?;
        if let Some(line) = rescale_after(&text, offset) {
            return Ok(line.to_string());
        }
        if Instant::now() >= deadline {
            return Err("DISPLAY-01: no DISPLAY_SCALE record after the Settings change".into());
        }
        thread::sleep(Duration::from_millis(80));
    }
}
/// Window-server and client evidence for one scale step.
fn step(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    keys: &[&str],
    (stored, effective): (u32, u32),
    capture: bool,
) -> Result<Value, String> {
    let offset = serial(&run.root, record)?.len();
    key(c, keys)?;
    let line = wait_rescale(run, record, offset)?;
    let number = |name: &str| field(&line, name).and_then(|v| v.parse::<u32>().ok());
    let logical = field(&line, "logical=")
        .ok_or("DISPLAY_SCALE logical")?
        .to_string();
    let expected_logical = format!(
        "{}x{}",
        logical_extent(OUTPUT.0, effective),
        logical_extent(OUTPUT.1, effective)
    );
    if number("preference=") != Some(stored)
        || number("effective=") != Some(effective)
        || number("dpi=") != Some(96 * effective / 100)
        || logical != expected_logical
    {
        return Err(format!(
            "DISPLAY-01: expected stored {stored} effective {effective} logical {expected_logical}: {line}"
        ));
    }
    let focus: Vec<i32> = field(&line, "focus=")
        .ok_or("DISPLAY_SCALE focus")?
        .split(',')
        .map(|v| v.parse().map_err(|_| "DISPLAY_SCALE focus value"))
        .collect::<Result<_, _>>()?;
    if focus.len() != 4 || focus[2] <= 0 || focus[3] <= 0 {
        return Err(format!(
            "DISPLAY-01: no focused window bounds after the rescale: {line}"
        ));
    }
    after_marker(
        run,
        record,
        offset,
        &format!("SETTINGS SAVED scale={stored} effective={effective}"),
    )?;
    after_marker(
        run,
        record,
        offset,
        &format!("DOOR_P12 CLIENT_SCALE percent={effective}"),
    )?;
    let text = serial(&run.root, record)?;
    let clients: Vec<String> = text
        .get(offset..)
        .unwrap_or("")
        .lines()
        .filter(|l| l.contains(&format!("DOOR_P12 CLIENT_SCALE percent={effective}")))
        .map(|l| l.split_whitespace().nth(1).unwrap_or("").to_string())
        .collect();
    thread::sleep(Duration::from_millis(800));
    if capture {
        let frame = scene::capture_settled(run, c, &format!("display01-{effective}"), &[])?;
        if (frame.width, frame.height) != (OUTPUT.0 as usize, OUTPUT.1 as usize) {
            return Err("DISPLAY-01: scanout extent changed with the scale".into());
        }
    }
    let factor = field(&line, "factor=").unwrap_or("").to_string();
    Ok(
        json!({"stored":stored,"effective":effective,"logical":logical,"focus_logical":focus,
        "factor":factor,"clients":clients,"atlas":if effective.is_multiple_of(100) {"hinted integer atlas"} else {"exported hinted fractional atlas"},
        "screenshot_scope":"artifact only"}),
    )
}
fn after_marker(run: &Run, record: &Value, offset: usize, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !serial(&run.root, record)?
        .get(offset..)
        .unwrap_or("")
        .contains(marker)
    {
        if Instant::now() >= deadline {
            return Err(format!("DISPLAY-01: {marker} missing"));
        }
        thread::sleep(Duration::from_millis(80));
    }
    Ok(())
}
fn page(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    query: &str,
    id: usize,
) -> Result<(), String> {
    let offset = serial(&run.root, record)?.len();
    key(c, &["esc"])?;
    key(c, &["ctrl", "f"])?;
    type_line(c, query)?;
    key(c, &["esc"])?;
    after_marker(run, record, offset, &format!("SETTINGS PAGE id={id}"))
}
/// A taskbar pixel (left of the centred buttons), physical coordinates.
fn taskbar_probe(frame: &scene::Pixels) -> Result<[u8; 3], String> {
    frame.pixel(40, frame.height - 20)
}
fn contrast(run: &mut Run, record: &Value, c: &mut qmp::Client) -> Result<Value, String> {
    let before = taskbar_probe(&scene::capture_settled(
        run,
        c,
        "display01-contrast-before",
        &[],
    )?)?;
    let mut states = Vec::new();
    for on in [true, false] {
        let offset = serial(&run.root, record)?.len();
        key(c, &["alt", "shift", "print"])?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let bits = loop {
            let text = serial(&run.root, record)?;
            if let Some(bits) = text.get(offset..).unwrap_or("").lines().find_map(|l| {
                l.split_once("DOOR_P12 CONTROLS saved bits=")
                    .and_then(|(_, v)| v.trim().parse::<u32>().ok())
            }) {
                break bits;
            }
            if Instant::now() >= deadline {
                return Err("DISPLAY-01: the contrast shortcut stored no controls".into());
            }
            thread::sleep(Duration::from_millis(80));
        };
        if (bits & CONTRAST_ON != 0) != on {
            return Err(format!(
                "DISPLAY-01: contrast {on} expected, stored bits {bits}"
            ));
        }
        thread::sleep(Duration::from_millis(1200));
        let probe = taskbar_probe(&scene::capture_settled(
            run,
            c,
            &format!("display01-contrast-{on}"),
            &[],
        )?)?;
        states.push(json!({"on":on,"bits":bits,"taskbar_probe":probe}));
        if on && probe == before {
            return Err("DISPLAY-01: the taskbar did not change under the contrast theme".into());
        }
        if !on && probe != before {
            return Err(
                "DISPLAY-01: the taskbar did not return to its colours after contrast off".into(),
            );
        }
    }
    Ok(json!({"shortcut":"Left Alt+Left Shift+Print Screen","before":before,"states":states}))
}
fn text_size(run: &Run, record: &Value, c: &mut qmp::Client) -> Result<Value, String> {
    page(run, record, c, "text size", TEXT_SIZE_PAGE)?;
    let mut stored = Vec::new();
    for (keys, percent) in [
        (
            &[
                &["tab"][..],
                &["pgup"],
                &["pgup"],
                &["right"],
                &["right"],
                &["right"],
                &["right"],
                &["right"],
                &["tab"],
            ][..],
            125u32,
        ),
        (&[&["shift", "tab"][..], &["home"], &["tab"]][..], 100),
    ] {
        for k in keys {
            key(c, k)?;
        }
        let offset = serial(&run.root, record)?.len();
        key(c, &["spc"])?;
        let bits = 48 | (percent - 100) << TEXT_SHIFT;
        after_marker(
            run,
            record,
            offset,
            &format!("SETTINGS SAVED controls={bits}"),
        )?;
        stored.push(json!({"percent":percent,"controls":bits}));
    }
    Ok(json!({"stored":stored,"applied_to_layout":"not verified by this suite (P12-TEXT-SIZE)"}))
}
fn drive(run: &mut Run, record: &Value) -> Result<Value, String> {
    let mut c = connection(record)?;
    thread::sleep(Duration::from_secs(2));
    let text = serial(&run.root, record)?;
    let density = text
        .lines()
        .find(|l| l.contains("DOOR_P12 DENSITY "))
        .ok_or("DISPLAY-01: DENSITY record missing")?
        .to_string();
    if field(&density, "percent=").and_then(|v| v.parse::<u32>().ok()) != Some(RECOMMENDED) {
        return Err(format!(
            "DISPLAY-01: expected the recommended {RECOMMENDED}% at sign-in: {density}"
        ));
    }
    resolution::activate(&run.root, record, &mut c, 2)?;
    page(run, record, &mut c, "display", DISPLAY_PAGE)?;
    for _ in 0..SCALE_TABS {
        key(&mut c, &["tab"])?;
    }
    let mut steps = Vec::new();
    // Down the offered list from the recommended 200%, back up to it (stored
    // as 0, automatic), then down to the persisted 150%.
    let mut plan: Vec<(&str, u32, u32)> = Vec::new();
    for &p in OFFERED.iter().rev().skip(1) {
        plan.push(("up", p, p));
    }
    for &p in &OFFERED[1..] {
        plan.push(("down", if p == RECOMMENDED { 0 } else { p }, p));
    }
    plan.push(("up", 175, 175));
    plan.push(("up", PERSISTED, PERSISTED));
    let mut captured = Vec::new();
    for (k, stored, effective) in plan {
        let capture = !captured.contains(&effective);
        captured.push(effective);
        steps.push(step(
            run,
            record,
            &mut c,
            &[k],
            (stored, effective),
            capture,
        )?);
    }
    // Bounds stay put in DIPs (one logical pixel of rounding per edge).
    let rects: Vec<Vec<i64>> = steps
        .iter()
        .map(|s| {
            s["focus_logical"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect()
        })
        .collect();
    for rect in &rects {
        if rect.iter().zip(&rects[0]).any(|(a, b)| (a - b).abs() > 1) {
            return Err(format!(
                "DISPLAY-01: focused window bounds changed in DIPs across scales: {rects:?}"
            ));
        }
    }
    let contrast = contrast(run, record, &mut c)?;
    let text_size = text_size(run, record, &mut c)?;
    start_shut_down(&mut c)?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P08 CLEANUP PASS",
        Duration::from_secs(30),
    )?;
    Ok(
        json!({"initial":density,"steps":steps,"dip_bounds":rects,"contrast":contrast,"text_size":text_size}),
    )
}
fn second_boot(run: &mut Run, record: &Value) -> Result<Value, String> {
    let mut c = connection(record)?;
    let bound =
        format!("DOOR_P12 DISPLAY_SCALE bound preference={PERSISTED} effective={PERSISTED}");
    wait_serial(&run.root, record, &bound, Duration::from_secs(60))?;
    resolution::activate(&run.root, record, &mut c, 2)?;
    let frame = scene::capture_settled(run, &mut c, "display01-second-boot", &[])?;
    start_shut_down(&mut c)?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P08 CLEANUP PASS",
        Duration::from_secs(30),
    )?;
    Ok(json!({"bound":bound,"scanout":[frame.width,frame.height]}))
}
/// Win+Ctrl+left Shift+F12: the window server's presentation fault chord,
/// acted on only where Init enabled its test hooks (the adversarial test
/// image, `system/window-tests.conf`).
const FAULT_CHORD: [&str; 4] = ["meta_l", "ctrl", "shift", "f12"];
/// Checked copies one chord fails (`door_window::scanout::INJECTED_COPIES`).
const INJECTED: usize = 3;
const FALLBACK_OUTPUT: (usize, usize) = (1024, 768);
fn records_after<'a>(text: &'a str, offset: usize, marker: &str) -> Vec<&'a str> {
    text.get(offset..)
        .unwrap_or("")
        .lines()
        .filter(|l| l.contains(marker))
        .collect()
}
/// Serial after the chord: one armed fault, `INJECTED` failed presentation
/// attempts with the 1, 2, 4 tick backoff, one recovery frame covering the
/// whole output, and no window-server restart (the session survived).
fn check_fallback(text: &str, offset: usize) -> Result<Value, String> {
    let armed = records_after(text, offset, "DOOR_P12 PRESENT_FAULT armed");
    if armed.len() != 1 || field(armed[0], "copies=") != Some(&INJECTED.to_string()) {
        return Err(format!("DISPLAY-01: fault chord not armed once: {armed:?}"));
    }
    let failed = records_after(text, offset, "DOOR_P12 PRESENT_FAILED ");
    let mut attempts = Vec::new();
    for (i, line) in failed.iter().enumerate() {
        let consecutive = field(line, "consecutive=").and_then(|v| v.parse::<usize>().ok());
        let retry = field(line, "retry_ticks=").and_then(|v| v.parse::<u64>().ok());
        if consecutive != Some(i + 1) || retry != Some(1 << i) || field(line, "kind=").is_none() {
            return Err(format!("DISPLAY-01: unexpected failure record: {line}"));
        }
        attempts.push(
            json!({"kind":field(line, "kind="),"consecutive":consecutive,"retry_ticks":retry}),
        );
    }
    if failed.len() != INJECTED {
        return Err(format!(
            "DISPLAY-01: expected {INJECTED} failed presentations, got {failed:?}"
        ));
    }
    let recovered = records_after(text, offset, "DOOR_P12 PRESENT_RECOVERED ");
    if recovered.len() != 1
        || field(recovered[0], "failures=") != Some(&INJECTED.to_string())
        || field(recovered[0], "full=") != Some("1")
    {
        return Err(format!(
            "DISPLAY-01: no single full-output recovery frame: {recovered:?}"
        ));
    }
    if !records_after(text, offset, "DOOR_P11 DISPLAY READY").is_empty()
        || text.contains("KERNEL_PANIC")
    {
        return Err("DISPLAY-01: the presentation failure restarted the window server".into());
    }
    Ok(json!({"attempts":attempts,"recovery":recovered[0],"window_server_restarted":false}))
}
/// DISPLAY-01 presentation failure and fallback, on the adversarial test
/// image (the only one whose Init enables the fault chord).
fn fallback_boot(run: &mut Run, record: &Value) -> Result<Value, String> {
    wait_serial(
        &run.root,
        record,
        "DOOR_P12 TEST_HOOKS present-fault=1",
        Duration::from_secs(120),
    )?;
    // Act once the adversarial workload waits for input, so the injected
    // failures overlap none of its checks.
    wait_serial(
        &run.root,
        record,
        "DOOR_P11 ADVERSARY ARMED",
        Duration::from_secs(120),
    )?;
    let mut c = connection(record)?;
    let offset = serial(&run.root, record)?.len();
    key(&mut c, &FAULT_CHORD)?;
    after_marker(run, record, offset, "DOOR_P12 PRESENT_RECOVERED")?;
    let mut evidence = check_fallback(&serial(&run.root, record)?, offset)?;
    let frame = scene::capture_settled(run, &mut c, "display01-fallback-recovered", &[])?;
    if (frame.width, frame.height) != FALLBACK_OUTPUT {
        return Err("DISPLAY-01: the recovered scanout changed extent".into());
    }
    // Input still reaches the shell: Start opens and is presented.
    let offset = serial(&run.root, record)?.len();
    key(&mut c, &["meta_l"])?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while records_after(
        &serial(&run.root, record)?,
        offset,
        "DOOR_P12 SCENE_PRESENTED",
    )
    .iter()
    .all(|l| !l.contains("overlay=1"))
    {
        if Instant::now() >= deadline {
            return Err("DISPLAY-01: Start was not presented after the fallback".into());
        }
        thread::sleep(Duration::from_millis(80));
    }
    let _ = c.execute("quit", json!({}));
    evidence["recovered_frame"] = json!([frame.width, frame.height]);
    evidence["input_after_recovery"] =
        json!("Start opened and presented (SCENE_PRESENTED overlay=1)");
    Ok(evidence)
}
pub(super) fn suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    let disk = run.directory.join("display01-user.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let resolution = format!("{}x{}", OUTPUT.0, OUTPUT.1);
    let mut first = session::launch_desktop_accelerated(run, image, &disk, false, &resolution)?;
    let result = drive(run, &first);
    if result.is_err()
        && let Ok(mut c) = connection(&first)
    {
        let _ = scene::capture_settled(run, &mut c, "display01-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    let mut evidence = result?;
    seal_serial(&run.root, &mut first)?;
    let mut second = session::launch_desktop_accelerated(run, image, &disk, false, &resolution)?;
    let result = second_boot(run, &second);
    if result.is_err()
        && let Ok(mut c) = connection(&second)
    {
        let _ = c.execute("quit", json!({}));
    }
    evidence["second_boot"] = result?;
    seal_serial(&run.root, &mut second)?;
    // Presentation failure: a third boot of the test image, whose Init alone
    // enables the window server's fault chord.
    let fallback_disk = run.directory.join("display01-fallback.raw");
    fs::write(&fallback_disk, crate::persistence::format::image(false))
        .map_err(|e| e.to_string())?;
    let mode = format!("{}x{}", FALLBACK_OUTPUT.0, FALLBACK_OUTPUT.1);
    let mut third =
        session::launch_desktop_accelerated(run, test_image, &fallback_disk, false, &mode)?;
    let result = fallback_boot(run, &third);
    if result.is_err()
        && let Ok(mut c) = connection(&third)
    {
        let _ = scene::capture_settled(run, &mut c, "display01-fallback-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    evidence["presentation_failure"] = result?;
    crate::evidence::seal_session_serial(&run.root, &mut third)?;
    evidence["sessions"] = json!([first, second, third]);
    evidence["method"] = json!(
        "QMP keyboard input; window server, client, Settings and Init serial records checked against host arithmetic; presentation failure injected by the test image's fault chord; screenshots are artifacts"
    );
    evidence["not_covered"] = json!([
        "output hotplug and mode-set failure (the CPU-copy GOP backend has neither path to fail)",
        "raster sharpness of text (the hinted atlas per scale is a code-path fact, not a pixel measurement)",
        "text size applied to layout"
    ]);
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display01_fallback_needs_backed_off_failures_one_full_recovery_and_no_restart() {
        let p = "DOOR_USER pid=9 DOOR_P12";
        let good = format!(
            "{p} DISPLAY READY early\n{p} PRESENT_FAULT armed copies=3\n\
             {p} PRESENT_FAILED kind=scene status=IoFailure consecutive=1 retry_ticks=1\n\
             {p} PRESENT_FAILED kind=cursor status=IoFailure consecutive=2 retry_ticks=2\n\
             {p} PRESENT_FAILED kind=cursor status=IoFailure consecutive=3 retry_ticks=4\n\
             {p} PRESENT_RECOVERED failures=3 full=1 total=3\n"
        );
        let offset = good.find(&format!("{p} PRESENT_FAULT")).unwrap();
        let evidence = check_fallback(&good, offset).unwrap();
        assert_eq!(evidence["attempts"].as_array().unwrap().len(), 3);
        for bad in [
            good.replace("full=1", "full=0"),
            good.replace("retry_ticks=4", "retry_ticks=3"),
            good.replace("failures=3", "failures=2"),
            good.replace("armed copies=3", "armed copies=2"),
            format!("{good}DOOR_USER pid=12 DOOR_P11 DISPLAY READY cpu-copy=1\n"),
            good.replace(
                &format!(
                    "{p} PRESENT_FAILED kind=cursor status=IoFailure consecutive=3 retry_ticks=4\n"
                ),
                "",
            ),
        ] {
            assert!(check_fallback(&bad, offset).is_err(), "{bad}");
        }
        assert_eq!(FAULT_CHORD, ["meta_l", "ctrl", "shift", "f12"]);
    }
    #[test]
    fn display01_expectations_follow_the_protocol_arithmetic() {
        assert_eq!(logical_extent(2560, 150), 1707);
        assert_eq!(logical_extent(1600, 150), 1067);
        assert_eq!(logical_extent(2560, 200), 1280);
        for p in OFFERED {
            assert!(logical_extent(OUTPUT.0, p) >= 1024 && logical_extent(OUTPUT.1, p) >= 600);
        }
        let line = "DOOR_USER pid=5 DOOR_P12 DISPLAY_SCALE preference=175 effective=175 dpi=168 logical=1463x914 factor=7/4 focus=10,20,800,600";
        let text = format!("x\n{line}\n");
        assert_eq!(rescale_after(&text, 0), Some(line));
        assert_eq!(field(line, "focus="), Some("10,20,800,600"));
    }
}
