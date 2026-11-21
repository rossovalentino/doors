//! P14 inbox apps (`p14-inbox`) on a private disk: Calculator starts from
//! Start search as its own kernel identity (manifest CALCULATOR, image
//! `door-inbox`), takes keyboard input (7 + 8 = shows 15, then 2 ÷ 0 = shows
//! the divide-by-zero message), is captured, and closes with Alt+F4, its
//! process exiting cleanly; Clock starts the same way (manifest CLOCK) and
//! closes cleanly.
use super::*;

/// Waits up to 20 s for `marker` in the serial log after byte `offset`.
fn fresh(run: &Run, record: &Value, offset: usize, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if serial(&run.root, record)?
            .get(offset..)
            .unwrap_or("")
            .contains(marker)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("inbox app marker missing: {marker}"));
        }
        thread::sleep(Duration::from_millis(80));
    }
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("p14-inbox-user.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, "1280x720")?;
    let result: Result<Value, String> = (|| {
        let root = run.root.clone();
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(2));
        let pid = super::gallery::launch(
            &root,
            &record,
            &mut c,
            "Calculator",
            "DOOR_INBOX READY app=calculator",
        )?;
        thread::sleep(Duration::from_millis(600));
        let before = serial(&root, &record)?.len();
        for keys in [&["7"][..], &["shift", "equal"], &["8"], &["ret"]] {
            key(&mut c, keys)?;
        }
        fresh(run, &record, before, "DOOR_INBOX CALCULATOR display=15")?;
        scene::capture_settled(run, &mut c, "p14-calculator-15", &[])?;
        let before = serial(&root, &record)?.len();
        for keys in [&["2"][..], &["slash"], &["0"], &["ret"]] {
            key(&mut c, keys)?;
        }
        fresh(
            run,
            &record,
            before,
            "DOOR_INBOX CALCULATOR display=Cannot divide by zero",
        )?;
        scene::capture_settled(run, &mut c, "p14-calculator-divide-by-zero", &[])?;
        let before = serial(&root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        fresh(
            run,
            &record,
            before,
            &format!("ACCESSORY EXIT pid={pid} code=0"),
        )?;
        // Clock: its own kernel identity (manifest CLOCK) from Start search,
        // with a taskbar button of its own while it runs (the shell's one
        // group), captured on its Timer page, closed with Alt+F4.
        let launched = serial(&root, &record)?.len();
        let clock = super::gallery::launch(
            &root,
            &record,
            &mut c,
            "Clock",
            "DOOR_INBOX READY app=clock",
        )?;
        fresh(run, &record, launched, "SHELL TASKBAR groups=1 windows=1")?;
        thread::sleep(Duration::from_millis(800));
        scene::capture_settled(run, &mut c, "p14-clock-timer", &[])?;
        let before = serial(&root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        fresh(
            run,
            &record,
            before,
            &format!("ACCESSORY EXIT pid={clock} code=0"),
        )?;
        let mut pointer = (640, 360);
        // Paint: a brush stroke across the canvas, then Alt+F4 asks to save
        // and "Don't save" (N) closes it.
        let paint = super::gallery::launch(
            &root,
            &record,
            &mut c,
            "Paint",
            "DOOR_INBOX READY app=paint",
        )?;
        thread::sleep(Duration::from_millis(800));
        scene::capture_settled(run, &mut c, "p14-paint", &[])?;
        move_to(&mut c, &mut pointer, 420, 380)?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
        )?;
        thread::sleep(Duration::from_millis(150));
        move_to(&mut c, &mut pointer, 860, 420)?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
        )?;
        thread::sleep(Duration::from_millis(400));
        scene::capture_settled(run, &mut c, "p14-paint-stroke", &[])?;
        let before = serial(&root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_millis(800));
        scene::capture_settled(run, &mut c, "p14-paint-save-prompt", &[])?;
        key(&mut c, &["n"])?;
        fresh(
            run,
            &record,
            before,
            &format!("ACCESSORY EXIT pid={paint} code=0"),
        )?;
        // Snipping Tool: New (Ctrl+N) opens the shell's snip overlay; a
        // dragged rectangle comes back to the app (clipboard), which saves
        // it to Pictures\Screenshots.
        let snipping = super::gallery::launch(
            &root,
            &record,
            &mut c,
            "Snipping Tool",
            "DOOR_INBOX READY app=snipping",
        )?;
        thread::sleep(Duration::from_millis(600));
        scene::capture_settled(run, &mut c, "p14-snipping", &[])?;
        let before = serial(&root, &record)?.len();
        key(&mut c, &["ctrl", "n"])?;
        fresh(run, &record, before, "DOOR_SNIP OPEN")?;
        thread::sleep(Duration::from_millis(1500));
        scene::capture_settled(run, &mut c, "p14-snip-overlay", &[])?;
        move_to(&mut c, &mut pointer, 200, 160)?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
        )?;
        thread::sleep(Duration::from_millis(150));
        move_to(&mut c, &mut pointer, 700, 460)?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
        )?;
        fresh(run, &record, before, "DOOR_SNIP TAKEN")?;
        fresh(run, &record, before, "DOOR_INBOX SNIP opened")?;
        thread::sleep(Duration::from_millis(1500));
        scene::capture_settled(run, &mut c, "p14-snip-open", &[])?;
        let before = serial(&root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        fresh(
            run,
            &record,
            before,
            &format!("ACCESSORY EXIT pid={snipping} code=0"),
        )?;
        // Weather, Narrator and Voice access open, show their state and close.
        for (query, ready, shot) in [
            ("Weather", "DOOR_INBOX READY app=weather", "p14-weather"),
            ("Narrator", "DOOR_INBOX READY app=narrator", "p14-narrator"),
            (
                "Voice access",
                "DOOR_INBOX READY app=voice",
                "p14-voice-access",
            ),
        ] {
            let pid = super::gallery::launch(&root, &record, &mut c, query, ready)?;
            thread::sleep(Duration::from_millis(800));
            scene::capture_settled(run, &mut c, shot, &[])?;
            let before = serial(&root, &record)?.len();
            key(&mut c, &["alt", "f4"])?;
            fresh(
                run,
                &record,
                before,
                &format!("ACCESSORY EXIT pid={pid} code=0"),
            )?;
        }
        // Magnifier: the shell magnifies the screen (200%, full screen) until
        // Magnifier closes.
        let before = serial(&root, &record)?.len();
        let magnifier = super::gallery::launch(
            &root,
            &record,
            &mut c,
            "Magnifier",
            "DOOR_INBOX READY app=magnifier",
        )?;
        fresh(run, &record, before, "DOOR_P14 MAGNIFY percent=200 view=0")?;
        thread::sleep(Duration::from_millis(800));
        scene::capture_settled(run, &mut c, "p14-magnifier", &[])?;
        let before = serial(&root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        fresh(
            run,
            &record,
            before,
            &format!("ACCESSORY EXIT pid={magnifier} code=0"),
        )?;
        // On-Screen Keyboard: a click on a key reaches the shell as a key
        // for the focused window; the keyboard itself never takes focus.
        let before = serial(&root, &record)?.len();
        let osk = super::gallery::launch(
            &root,
            &record,
            &mut c,
            "On-Screen Keyboard",
            "DOOR_INBOX READY app=osk",
        )?;
        thread::sleep(Duration::from_millis(800));
        scene::capture_settled(run, &mut c, "p14-osk", &[])?;
        click(&mut c, &mut pointer, 575, 380)?;
        fresh(run, &record, before, "DOOR_INBOX OSK key")?;
        if !serial(&root, &record)?
            .get(before..)
            .unwrap_or("")
            .contains("ok=1")
        {
            return Err("the On-Screen Keyboard's key was refused".into());
        }
        // The keyboard has no focus to close with Alt+F4: its close button.
        let before = serial(&root, &record)?.len();
        // (The window is 896 px wide: 70% of the 1280 px output.)
        click(&mut c, &mut pointer, 1065, 187)?;
        if fresh(
            run,
            &record,
            before,
            &format!("ACCESSORY EXIT pid={osk} code=0"),
        )
        .is_err()
        {
            key(&mut c, &["alt", "f4"])?;
            fresh(
                run,
                &record,
                before,
                &format!("ACCESSORY EXIT pid={osk} code=0"),
            )?;
        }
        // The modern Run dialog (Win+R): its bar, suggestions while typing,
        // then `calc` starts Calculator, `control` opens Settings and
        // `~\Documents` opens File Explorer there.
        let typed = |c: &mut qmp::Client, text: &str| -> Result<(), String> {
            for ch in text.chars() {
                c.execute(
                    "send-key",
                    json!({"keys":session::keys(ch)?,"hold-time":20}),
                )?;
                thread::sleep(Duration::from_millis(60));
            }
            Ok(())
        };
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(500));
        scene::capture_settled(run, &mut c, "p14-run", &[])?;
        typed(&mut c, "calc")?;
        thread::sleep(Duration::from_millis(300));
        scene::capture_settled(run, &mut c, "p14-run-typing", &[])?;
        let before = serial(&root, &record)?.len();
        key(&mut c, &["ret"])?;
        fresh(run, &record, before, "DOOR_P12 RUN Calculator")?;
        fresh(run, &record, before, "DOOR_INBOX READY app=calculator")?;
        thread::sleep(Duration::from_millis(600));
        let closing = serial(&root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        fresh(run, &record, closing, "ACCESSORY EXIT pid=")?;
        let before = serial(&root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        typed(&mut c, "control")?;
        key(&mut c, &["ret"])?;
        fresh(run, &record, before, "DOOR_P12 SETTINGS_LINK page=0")?;
        thread::sleep(Duration::from_millis(1500));
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_millis(800));
        let before = serial(&root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        typed(&mut c, "~\\Documents")?;
        key(&mut c, &["ret"])?;
        fresh(
            run,
            &record,
            before,
            "DOOR_P12 FILES ADDRESS path=/users/1001/Documents",
        )?;
        thread::sleep(Duration::from_millis(800));
        scene::capture_settled(run, &mut c, "p14-run-documents", &[])?;
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_millis(800));
        // A clean shutdown, so the final kernel cleanup records are complete.
        start_shut_down(&mut c)?;
        wait_serial(
            &root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(20),
        )?;
        Ok(
            json!({"calculator_pid":pid,"keyboard":["7 + 8 = 15","2 / 0 = Cannot divide by zero"],"clock_pid":clock,"closed":"Alt+F4, exit 0"}),
        )
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = scene::capture_settled(run, &mut c, "p14-inbox-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    let mut evidence = result?;
    seal_serial(&run.root, &mut record)?;
    let text = serial(&run.root, &record)?;
    if text.contains("DOOR_INBOX FAILED") || text.contains("KERNEL_PANIC") {
        return Err("inbox app failure in the serial log".into());
    }
    evidence["session"] = record;
    run.check("p14-inbox-calculator", evidence);
    Ok(())
}
