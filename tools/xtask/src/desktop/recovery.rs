//! Separate malicious client and real display-server death. The production
//! image has neither the probe executable nor its activation file.
use super::*;
/// Initial compositor only: each accepted bind must complete its own full scene
/// with a fresh material before the next form can repair/mask its result.
pub(super) fn binding_presentations(text: &str) -> Result<Value, String> {
    let (density_line, pid) = text
        .lines()
        .enumerate()
        .find_map(|(index, line)| {
            let (pid, payload) = line.strip_prefix("DOOR_USER pid=")?.split_once(' ')?;
            payload
                .starts_with("DOOR_P12 DENSITY ")
                .then_some((index, pid))
        })
        .ok_or("binding compositor identity absent")?;
    let number = pid.parse::<u64>().map_err(|_| "binding compositor PID")?;
    if number.to_string() != pid {
        return Err("noncanonical binding compositor PID".into());
    }
    let processes: Vec<_> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| line.starts_with("DOOR_P08 PROCESS "))
        .map(|(index, line)| {
            crate::services::scenarios::process(line)
                .map(|process| (index, process))
                .ok_or("malformed kernel process identity")
        })
        .collect::<Result<_, _>>()?;
    let matching: Vec<_> = processes
        .iter()
        .filter(|(_, process)| process.pid == number)
        .collect();
    if matching.len() != 1 {
        return Err("one retired compositor identity required".into());
    }
    let (exit_line, process) = matching[0];
    if number == 0
        || process.instance == 0
        || process.manifest != 11
        || process.uid != 3
        || process.session != 0
        || process.epoch != 0
        || process.exit != -311
        || density_line >= *exit_line
        || processes
            .iter()
            .filter(|(_, p)| p.instance == process.instance)
            .count()
            != 1
    {
        return Err("binding records lack the expected kernel compositor incarnation".into());
    }
    let prefix = format!("DOOR_USER pid={pid} DOOR_P12 SESSION_PRESENTED ");
    let records: Vec<_> = text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| line.strip_prefix(&prefix).map(|payload| (index, payload)))
        .collect();
    if records.len() != 3 {
        return Err("three initial compositor binding presentations required".into());
    }
    let mut frames = Vec::new();
    let mut previous = 0;
    for ((index, record), bytes) in records.into_iter().zip([24, 32, 40]) {
        if index <= density_line || index >= *exit_line {
            return Err("binding presentation lies outside compositor lifetime".into());
        }
        let fields: Vec<_> = record.split(' ').collect();
        if fields.len() != 5
            || fields[0] != format!("bind_bytes={bytes}")
            || fields[1] != "login_presented=1"
            || fields[2] != "full=1"
            || fields[3] != "material=1"
        {
            return Err(
                "binding did not replace the presented login with a fresh full scene".into(),
            );
        }
        let frame = fields[4]
            .strip_prefix("frame=")
            .ok_or("binding frame field")?
            .parse::<u64>()
            .map_err(|_| "binding frame value")?;
        if frame <= previous || fields[4] != format!("frame={frame}") {
            return Err("binding presentations must be distinct ordered completed frames".into());
        }
        previous = frame;
        frames
            .push(json!({"bind_bytes":bytes,"frame":frame,"full":true,"material_refreshed":true}));
    }
    Ok(
        json!({"pid":number,"instance":process.instance,"manifest":process.manifest,"presentations":frames,"scope":"Kernel-attributed completed initial compositor frames after a presented login; each protocol form refreshes the full scene and taskbar material"}),
    )
}
pub(super) fn validate_binding_summary(text: &str, checks: &[Value]) -> Result<(), String> {
    let summaries: Vec<_> = checks
        .iter()
        .filter(|c| c["id"] == "desktop-session-bind-rendering")
        .collect();
    let declared = text.lines().any(|line| {
        line.starts_with("DOOR_USER pid=") && line.contains(" DOOR_P12 SESSION_PRESENTED ")
    });
    if !declared && summaries.is_empty() {
        return Ok(());
    }
    if summaries.len() != 1
        || summaries[0]["outcome"] != "passed"
        || summaries[0]["details"] != binding_presentations(text)?
    {
        return Err("binding presentation summary differs from sealed native serial".into());
    }
    Ok(())
}
fn held_foreign_request(run: &Run, record: &Value, c: &mut qmp::Client) -> Result<(), String> {
    let before = serial(&run.root, record)?.len();
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !serial(&run.root, record)?
        .get(before..)
        .unwrap_or("")
        .contains("FOREIGN_CLOSE_DENIED")
    {
        if Instant::now() >= deadline {
            return Err("no rejected foreign close during held gesture".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
fn release(c: &mut qmp::Client) -> Result<(), String> {
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
    )?;
    Ok(())
}
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("desktop-recovery.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    // This bounded checkpoint uses the pinned WHPX/RAMFB product backend. The
    // historical TCG recovery attempt/deadline failure remains separate evidence.
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, "1024x768")?;
    let result = (|| {
        wait_serial(
            &run.root,
            &record,
            "DOOR_P11 ADVERSARY ARMED",
            // This marker follows hundreds of RPCs, thread/snapshot checks and
            // 32 presented popup lifecycles under CPU-emulated graphics.
            Duration::from_secs(120),
        )?;
        let mut c = connection(&record)?;
        let mut pointer = (512, 384);
        // Points from the real authority: Explorer and Notes share their
        // initial placement under the probe's windows (`window_points`).
        let r = window_points::recovery()?;
        // Explicit foreground routing: Notes covers Explorer exactly here.
        resolution::activate(&run.root, &record, &mut c, 0)?;
        move_to(&mut c, &mut pointer, r.drag[0].0, r.drag[0].1)?;
        held_foreign_request(run, &record, &mut c)?;
        move_to(&mut c, &mut pointer, r.drag[1].0, r.drag[1].1)?;
        release(&mut c)?;
        let (bx, by) = r.moved_border;
        scene::capture(
            run,
            &mut c,
            "foreign-close-keeps-drag",
            // The moved window's right border column lies outside its
            // placement and the probe windows: wallpaper unless it moved.
            &[(bx, by, window_points::border())],
        )?;
        move_to(&mut c, &mut pointer, r.drag_back[0].0, r.drag_back[0].1)?;
        held_foreign_request(run, &record, &mut c)?;
        move_to(&mut c, &mut pointer, r.drag_back[1].0, r.drag_back[1].1)?;
        release(&mut c)?;
        move_to(&mut c, &mut pointer, r.maximize.0, r.maximize.1)?;
        held_foreign_request(run, &record, &mut c)?;
        release(&mut c)?;
        let (sx, sy) = r.max_status;
        scene::capture(
            run,
            &mut c,
            "foreign-close-keeps-caption",
            // Explorer's status bar beyond its placement and the probe
            // windows: present only once Explorer itself is maximized.
            &[(sx, sy, window_points::FILES_STATUS)],
        )?;
        // Minimize the maximized Files client to expose Notes underneath.
        click(&mut c, &mut pointer, r.minimize.0, r.minimize.1)?;
        resolution::activate(&run.root, &record, &mut c, 1)?;
        click(&mut c, &mut pointer, r.notes_body.0, r.notes_body.1)?;
        type_line(&mut c, "Before restart")?;
        // Leave the document unsaved, then terminate the independent probe via
        // ordinary keyboard input. Init injects service loss only after it exits.
        // Minimize the overlapping editor without closing its unsaved buffer,
        // then target the actual probe client instead of a covered coordinate.
        key(&mut c, &["meta_l", "down"])?;
        click(&mut c, &mut pointer, r.probe.0, r.probe.1)?;
        key(&mut c, &["esc"])?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P11 ADVERSARY PASS",
            Duration::from_secs(45),
        )?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P11 HANG ARMED",
            Duration::from_secs(30),
        )?;
        let text = serial(&run.root, &record)?;
        run.check(
            "desktop-session-bind-rendering",
            binding_presentations(&text)?,
        );
        if !text.contains("DOOR_P11 RECOVERY INJECT")
            || !text.contains("restart=1")
            || !reconnected_both(&text)
        {
            return Err("display restart did not preserve/reconnect both independent apps".into());
        }
        // Display restart resets the emulated device pointer to its center.
        pointer = (512, 384);
        // Reconnection notices precede model recreation/first scanout. Wait for
        // both actual client presentations, then minimize without discarding
        // their models so neither can cover the independent hung producer.
        for index in [0, 1] {
            resolution::activate(&run.root, &record, &mut c, index)?;
            key(&mut c, &["meta_l", "down"])?;
        }
        scene::capture_settled(run, &mut c, "recovery-hung-producer-exposed", &[])?;
        click(&mut c, &mut pointer, r.hung_close.0, r.hung_close.1)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P11 HUNG_CLOSE PASS",
            Duration::from_secs(15),
        )?;
        resolution::activate(&run.root, &record, &mut c, 1)?;
        click(&mut c, &mut pointer, r.notes_body.0, r.notes_body.1)?;
        type_line(&mut c, "Recovered desktop note")?;
        key(&mut c, &["ctrl", "s"])?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P11 NOTES SAVED",
            Duration::from_secs(15),
        )?;
        let screenshot = run.directory.join("desktop-recovered.png");
        c.execute("screendump",json!({"filename":screenshot.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/"),"format":"png"}))?;
        shutdown(&run.root, &record, &mut c, &mut pointer)?;
        crate::evidence::seal_session_serial(&run.root, &mut record)?;
        let independent = audit(&disk, b"Before restart\nRecovered desktop note\n")?;
        run.check("desktop-recovery-suite",json!({"outcome":"passed","image":image,"record":record,"audit":independent,"screenshot":relative(&run.root,&screenshot)?,"screenshot_sha256":file_hash(&screenshot)?,"adversary":true,"real_service_loss":true,"hung_client_terminated":true}));
        Ok(())
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = c.execute("quit", json!({}));
    }
    result
}
#[cfg(test)]
mod binding_tests {
    use super::*;
    fn serial() -> String {
        let mut text = String::from("DOOR_USER pid=5 DOOR_P12 DENSITY physical=1024x768\n");
        for (bytes, frame) in [(24, 2), (32, 3), (40, 4)] {
            text.push_str(&format!("DOOR_USER pid=5 DOOR_P12 SESSION_PRESENTED bind_bytes={bytes} login_presented=1 full=1 material=1 frame={frame}\n"));
        }
        text.push_str(
            "DOOR_P08 PROCESS manifest=11 pid=5 instance=5 uid=3 session=0 epoch=0 exit=-311\n",
        );
        text
    }
    #[test]
    fn binding_oracle_requires_each_real_full_completed_frame_and_exact_summary() {
        let text = serial();
        let record = binding_presentations(&text).unwrap();
        assert_eq!(record["presentations"].as_array().unwrap().len(), 3);
        for bad in [
            text.replace("bind_bytes=24", "bind_bytes=40"),
            text.replace("login_presented=1", "login_presented=0"),
            text.replace("full=1", "full=0"),
            text.replace("material=1", "material=0"),
            text.replace("frame=3", "frame=2"),
            text.replace("frame=3", "frame=03"),
            text.replace("pid=5 DOOR_P12 SESSION", "pid=6 DOOR_P12 SESSION"),
            text.replace("DOOR_USER pid=5", "DOOR_USER pid=6"),
            text.replace("DOOR_USER pid=5", "DOOR_USER pid=0"),
            text.replace("DOOR_USER pid=5", "DOOR_USER pid=05"),
            text.replace("manifest=11", "manifest=12"),
            text.replace("uid=3", "uid=1001"),
            text.replace("session=0", "session=1"),
            text.replace("epoch=0", "epoch=1"),
            text.replace("exit=-311", "exit=-1"),
            text.replace("instance=5", "instance=0"),
            text.replace("instance=5", "instance=05"),
            text.lines()
                .filter(|line| !line.starts_with("DOOR_P08 PROCESS"))
                .collect::<Vec<_>>()
                .join("\n"),
            text.replace("material=1 frame=3", "material=1 extra=1 frame=3"),
            text.replace("bind_bytes=32", "bind_bytes=32 "),
            format!("{text}{}", text.lines().nth(1).unwrap()),
            format!("{text}{}", text.lines().last().unwrap()),
            format!(
                "{}\n{}",
                text.lines().last().unwrap(),
                text.lines().take(4).collect::<Vec<_>>().join("\n")
            ),
            format!(
                "{text}DOOR_P08 PROCESS manifest=12 pid=6 instance=5 uid=1001 session=1 epoch=1 exit=0\n"
            ),
        ] {
            assert!(binding_presentations(&bad).is_err());
        }
    }
    #[test]
    fn new_binding_markers_require_one_exact_summary_while_historical_serial_does_not() {
        let text = serial();
        let good = json!({"id":"desktop-session-bind-rendering","outcome":"passed","details":binding_presentations(&text).unwrap()});
        validate_binding_summary(&text, core::slice::from_ref(&good)).unwrap();
        validate_binding_summary("Historical serial with no binding presentations", &[]).unwrap();
        assert!(validate_binding_summary(&text, &[]).is_err());
        assert!(validate_binding_summary(&text, &[good.clone(), good.clone()]).is_err());
        let mut changed = good.clone();
        changed["details"]["instance"] = json!(6);
        assert!(validate_binding_summary(&text, &[changed]).is_err());
        let mut changed = good.clone();
        changed["outcome"] = json!("failed");
        assert!(validate_binding_summary(&text, &[changed]).is_err());
        let mut changed = good;
        changed["details"] = Value::Null;
        assert!(validate_binding_summary(&text, &[changed]).is_err());
    }
}
