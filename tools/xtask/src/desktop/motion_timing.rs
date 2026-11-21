//! MOTION-01 (`cargo xtask test ui --suite motion-01`): animation timing,
//! interruption, reduced motion and idle frames, measured guest-side. The
//! window server prints one `DOOR_P12 ANIM` record per settled animation
//! episode (`door_window::telemetry::AnimationTrace`): kinds, start on the
//! compositor's transition clock, the last in-progress frame, the rest frame
//! (composed and presented) and a running frame count. Durations come only
//! from those guest timestamps; QMP is used for input, never for timing.
//!
//! The specification names no numeric Windows 11 26H2 durations (section 111
//! requires a measured capture). The comparison targets are Door's motion
//! tokens: Start open is calibrated against the captured 26H2 sequence
//! (VIS-01 `start.open`), the other tracks remain provisional.
//! Records are armed by turning Animation effects off and on once, so other
//! workflows spend none of the window server's diagnostic credit on them.
use super::*;

const VISUAL_EFFECTS_PAGE: usize = 43;
/// `door_window::telemetry::ANIMATION_*` and `door_graphics_core::TRANSITION_*`.
const START: u64 = 1;
const OPEN: u64 = 1 << 3;
const MINIMIZE: u64 = 1 << 5;
const RESTORE: u64 = 1 << 6;
const RELOCATE: u64 = 1 << 7;
/// Door motion tokens (ms), read from the same constants the guest uses:
/// Start and the tray flyouts (`door_window::motion`, from the
/// `door_design_tokens` StartOpen/StartClose tracks) and window transitions
/// (`door_graphics_core` WINDOW_*_MS, from the WindowOpen/Minimize/Restore/
/// Maximize tracks).
const START_OPEN_MS: u64 = door_window::motion::OPEN_US / 1000;
const START_CLOSE_MS: u64 = door_window::motion::CLOSE_US / 1000;
const WINDOW_OPEN_MS: u64 = door_window::graphics::WINDOW_OPEN_MS;
const MINIMIZE_MS: u64 = door_window::graphics::WINDOW_MINIMIZE_MS;
const RESTORE_MS: u64 = door_window::graphics::WINDOW_RESTORE_MS;
const SNAP_MS: u64 = door_window::graphics::WINDOW_SNAP_MS;
const REFERENCE_SOURCE: &str = "Door motion token (spec section 111; Fluent motion curves and duration steps): Start open is calibrated against reports/artifacts/reference-26h2/motion/start-open/sequence.json (VIS-01 start.open); the other tracks are bootstrap proposals";
/// Extra time a rest frame may take beyond its token before the run fails:
/// the larger of 250 ms and three mean frame intervals of the episode.
const SLACK_MS: u64 = 250;
/// Frames tolerated between a rest frame and the next episode across 3 s of
/// idle: focus/taskbar settling right after the minimize plus a clock tick.
const IDLE_FRAMES: u64 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Anim {
    kinds: u64,
    start: u64,
    last: u64,
    rest: u64,
    presented: u64,
    moving: u64,
    frame: u64,
}
fn parse(line: &str) -> Option<Anim> {
    let (_, payload) = line.split_once("DOOR_P12 ANIM ")?;
    let mut fields = payload.split_whitespace();
    let mut next = |name: &str, hex: bool| -> Option<u64> {
        let value = fields.next()?.strip_prefix(name)?.strip_prefix('=')?;
        if hex {
            u64::from_str_radix(value, 16).ok()
        } else {
            value.parse().ok()
        }
    };
    let anim = Anim {
        kinds: next("k", true)?,
        start: next("t", false)?,
        last: next("l", false)?,
        rest: next("e", false)?,
        presented: next("p", false)?,
        moving: next("f", false)?,
        frame: next("n", false)?,
    };
    (anim.moving > 0 && anim.last <= anim.rest && anim.rest <= anim.presented).then_some(anim)
}
fn records(text: &str, offset: usize) -> Vec<Anim> {
    text.get(offset..)
        .unwrap_or("")
        .lines()
        .filter_map(parse)
        .collect()
}
fn json_of(a: &Anim) -> Value {
    json!({"kinds":a.kinds,"start_ms":a.start,"last_in_progress_ms":a.last,"rest_composed_ms":a.rest,
        "rest_presented_ms":a.presented,"in_progress_frames":a.moving,"frame":a.frame})
}
/// Fails on clear violations of a single, uninterrupted transition.
fn judge(name: &str, a: &Anim, reference: u64) -> Result<Value, String> {
    let interval = a.last / a.moving.max(1);
    let slack = SLACK_MS.max(3 * interval);
    if a.rest < reference {
        return Err(format!(
            "MOTION-01 {name}: rest frame at {} ms, before the {reference} ms track ended (not time-based)",
            a.rest
        ));
    }
    if a.rest > reference + slack {
        return Err(format!(
            "MOTION-01 {name}: rest frame at {} ms, beyond {reference} ms + {slack} ms",
            a.rest
        ));
    }
    Ok(
        json!({"name":name,"record":json_of(a),"reference_ms":reference,"reference_source":REFERENCE_SOURCE,
        "measured_rest_ms":a.rest,"measured_presented_ms":a.presented,"delta_ms":a.rest as i64 - reference as i64,
        "mean_frame_interval_ms":interval,"slack_ms":slack}),
    )
}
struct Motion<'a> {
    root: std::path::PathBuf,
    record: &'a Value,
}
impl Motion<'_> {
    fn text(&self) -> Result<String, String> {
        serial(&self.root, self.record)
    }
    fn offset(&self) -> Result<usize, String> {
        Ok(self.text()?.len())
    }
    /// Waits until `marker` follows `offset`.
    fn marker(&self, offset: usize, marker: &str) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.text()?.get(offset..).unwrap_or("").contains(marker) {
            if Instant::now() >= deadline {
                return Err(format!("MOTION-01: {marker} missing"));
            }
            thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }
    /// The first record after `offset` containing every bit of `kinds`.
    fn anim(&self, offset: usize, kinds: u64, what: &str) -> Result<Anim, String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(a) = records(&self.text()?, offset)
                .into_iter()
                .find(|a| a.kinds & kinds == kinds)
            {
                return Ok(a);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "MOTION-01 {what}: no settled animation record with kinds {kinds:#x}"
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    /// QMP input, then the record and the final-state marker.
    fn episode(
        &self,
        c: &mut qmp::Client,
        keys: &[&str],
        kinds: u64,
        reference: u64,
        name: &str,
        final_state: Option<&str>,
    ) -> Result<Value, String> {
        let offset = self.offset()?;
        key(c, keys)?;
        let a = self.anim(offset, kinds, name)?;
        if let Some(state) = final_state {
            self.marker(offset, state)?;
        }
        let mut verdict = judge(name, &a, reference)?;
        verdict["final_state"] = json!(final_state);
        thread::sleep(Duration::from_millis(400));
        Ok(verdict)
    }
}
/// A key pressed twice `gap` apart, without the 200 ms settle of `key`.
fn double(c: &mut qmp::Client, keys: &[&str], gap: Duration) -> Result<(), String> {
    let keys: Vec<Value> = keys
        .iter()
        .map(|k| json!({"type":"qcode","data":k}))
        .collect();
    c.execute("send-key", json!({"keys":keys,"hold-time":25}))?;
    thread::sleep(gap);
    c.execute("send-key", json!({"keys":keys,"hold-time":25}))?;
    Ok(())
}
fn set_reduce_motion(m: &Motion, c: &mut qmp::Client, on: bool) -> Result<(), String> {
    let offset = m.offset()?;
    key(c, &["esc"])?;
    key(c, &["ctrl", "f"])?;
    type_line(c, "visual effects")?;
    key(c, &["esc"])?;
    m.marker(offset, &format!("SETTINGS PAGE id={VISUAL_EFFECTS_PAGE}"))?;
    let offset = m.offset()?;
    key(c, &["tab"])?;
    key(c, &["tab"])?;
    key(c, &["spc"])?;
    m.marker(
        offset,
        &format!(
            "SETTINGS SAVED controls={}",
            if on { 48 | 4096 } else { 48 }
        ),
    )?;
    // Settings forwards the controls to the window server from its loop.
    thread::sleep(Duration::from_millis(1500));
    Ok(())
}
fn drive(run: &mut Run, record: &Value) -> Result<Value, String> {
    let m = Motion {
        root: run.root.clone(),
        record,
    };
    let mut c = connection(record)?;
    thread::sleep(Duration::from_secs(2));
    // Arm the window server's records: Animation effects off, then on
    // (`AnimationTrace::preference`); then close Settings.
    resolution::activate(&run.root, record, &mut c, 2)?;
    set_reduce_motion(&m, &mut c, true)?;
    // Start and Settings already animated, unarmed: nothing was printed.
    if !records(&m.text()?, 0).is_empty() {
        return Err("MOTION-01: animation records before the trace was armed".into());
    }
    set_reduce_motion(&m, &mut c, false)?;
    let offset = m.offset()?;
    key(&mut c, &["alt", "f4"])?;
    m.marker(offset, "DOOR_P11 CLIENT_EXIT")?;
    thread::sleep(Duration::from_millis(800));
    let boot = m.offset()?;
    resolution::activate(&run.root, record, &mut c, 1)?;
    thread::sleep(Duration::from_millis(800));
    // Notes' open transition starts at its first frame, usually while Start
    // is still closing; report it, judge it only when it settled alone.
    let opened = m.anim(boot, OPEN, "Notes window open")?;
    let window_open = if opened.kinds == OPEN {
        judge("window-open", &opened, WINDOW_OPEN_MS)?
    } else {
        json!({"name":"window-open","record":json_of(&opened),"reference_ms":WINDOW_OPEN_MS,"judged":false,
            "reason":"overlapped with another transition in one episode"})
    };
    let mut measured = vec![window_open];
    measured.push(m.episode(
        &mut c,
        &["meta_l"],
        START,
        START_OPEN_MS,
        "start-open",
        Some("overlay=1"),
    )?);
    measured.push(m.episode(
        &mut c,
        &["esc"],
        START,
        START_CLOSE_MS,
        "start-close",
        Some("SCENE_PRESENTED app=2 overlay=0"),
    )?);
    // Interrupted: Start opened, then closed about 100 ms into its entrance.
    let offset = m.offset()?;
    double(&mut c, &["meta_l"], Duration::from_millis(80))?;
    let reversed = m.anim(offset, START, "start-interrupted")?;
    m.marker(offset, "SCENE_PRESENTED app=2 overlay=0")?;
    thread::sleep(Duration::from_millis(600));
    let start_records = records(&m.text()?, offset)
        .iter()
        .filter(|a| a.kinds & START != 0)
        .count();
    if start_records != 1 || reversed.rest >= START_OPEN_MS + START_CLOSE_MS + SLACK_MS {
        return Err(format!(
            "MOTION-01 start-interrupted: expected one reversed episode, got {start_records} records ({reversed:?})"
        ));
    }
    let interrupted_start = json!({"record":json_of(&reversed),"records":start_records,
        "bound_ms":START_OPEN_MS + START_CLOSE_MS,"final_state":"Start closed, Notes foreground (SCENE_PRESENTED app=2 overlay=0)"});
    // Win+Up glides to the maximized bounds; Win+Down restores (not animated).
    measured.push(m.episode(
        &mut c,
        &["meta_l", "up"],
        RELOCATE,
        SNAP_MS,
        "maximize",
        None,
    )?);
    let offset = m.offset()?;
    key(&mut c, &["meta_l", "down"])?;
    thread::sleep(Duration::from_millis(1500));
    let restore_down = records(&m.text()?, offset);
    measured.push(m.episode(
        &mut c,
        &["meta_l", "left"],
        RELOCATE,
        SNAP_MS,
        "snap-left",
        None,
    )?);
    key(&mut c, &["esc"])?;
    let offset = m.offset()?;
    key(&mut c, &["meta_l", "down"])?;
    thread::sleep(Duration::from_millis(1500));
    let unsnap = records(&m.text()?, offset);
    // Minimize to the taskbar button (Win+2: Notes), idle, restore.
    measured.push(m.episode(
        &mut c,
        &["meta_l", "2"],
        MINIMIZE,
        MINIMIZE_MS,
        "minimize",
        Some("SCENE_PRESENTED app=0 overlay=0"),
    )?);
    // The newest rest frame so far is the minimize episode's.
    let last_rest = records(&m.text()?, 0)
        .iter()
        .map(|a| a.frame)
        .max()
        .ok_or("MOTION-01: no animation records")?;
    thread::sleep(Duration::from_secs(3));
    let offset = m.offset()?;
    key(&mut c, &["meta_l", "2"])?;
    let restored = m.anim(offset, RESTORE, "restore")?;
    m.marker(offset, "SCENE_PRESENTED app=2 overlay=0")?;
    let idle_frames = (restored.frame - restored.moving).saturating_sub(last_rest + 1);
    if idle_frames > IDLE_FRAMES {
        return Err(format!(
            "MOTION-01: {idle_frames} frames presented during 3 s idle with nothing animating"
        ));
    }
    measured.push(judge("restore", &restored, RESTORE_MS)?);
    thread::sleep(Duration::from_millis(400));
    // Interrupted: minimize reversed by restore about 100 ms in.
    let offset = m.offset()?;
    double(&mut c, &["meta_l", "2"], Duration::from_millis(80))?;
    let both = m.anim(offset, MINIMIZE | RESTORE, "minimize-interrupted")?;
    m.marker(offset, "SCENE_PRESENTED app=2 overlay=0")?;
    if both.rest >= MINIMIZE_MS + RESTORE_MS + SLACK_MS {
        return Err(format!("MOTION-01 minimize-interrupted: {both:?}"));
    }
    let interrupted_window = json!({"record":json_of(&both),"bound_ms":MINIMIZE_MS + RESTORE_MS,
        "final_state":"Notes restored and foreground (SCENE_PRESENTED app=2 overlay=0)"});
    // Reduced motion: no animation records, every final state reached.
    resolution::activate(&run.root, record, &mut c, 2)?;
    set_reduce_motion(&m, &mut c, true)?;
    let offset = m.offset()?;
    key(&mut c, &["meta_l"])?;
    m.marker(offset, "overlay=1")?;
    let closed = m.offset()?;
    key(&mut c, &["esc"])?;
    m.marker(closed, "SCENE_PRESENTED app=3 overlay=0")?;
    let minimized_at = m.offset()?;
    key(&mut c, &["meta_l", "3"])?;
    m.marker(minimized_at, "SCENE_PRESENTED app=0 overlay=0")?;
    let restored_at = m.offset()?;
    key(&mut c, &["meta_l", "3"])?;
    m.marker(restored_at, "SCENE_PRESENTED app=3 overlay=0")?;
    thread::sleep(Duration::from_millis(1000));
    let reduced = records(&m.text()?, offset);
    if !reduced.is_empty() {
        return Err(format!(
            "MOTION-01: animation records under reduced motion: {reduced:?}"
        ));
    }
    set_reduce_motion(&m, &mut c, false)?;
    let resumed = m.episode(
        &mut c,
        &["meta_l"],
        START,
        START_OPEN_MS,
        "start-open-after-reduced",
        Some("overlay=1"),
    )?;
    key(&mut c, &["esc"])?;
    thread::sleep(Duration::from_millis(600));
    start_shut_down(&mut c)?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P08 CLEANUP PASS",
        Duration::from_secs(30),
    )?;
    let text = m.text()?;
    let emitted = records(&text, 0).len();
    Ok(
        json!({"measured":measured,"interrupted_start":interrupted_start,"interrupted_window":interrupted_window,
        "maximize_restore_down":{"records":restore_down.iter().map(json_of).collect::<Vec<_>>(),
            "note":"Win+Down from maximized restores without a transition in Door; Windows animates it (fidelity deviation, not judged)"},
        "unsnap":unsnap.iter().map(json_of).collect::<Vec<_>>(),
        "idle":{"seconds":3,"frames_presented":idle_frames,"limit":IDLE_FRAMES,"includes":"taskbar/focus settling frames after the minimize and at most one clock update"},
        "reduced_motion":{"controls":4144,"records":0,"final_states":["Start opened","Start closed","Settings minimized","Settings restored"]},
        "after_reduced_motion":resumed,"records_emitted":emitted}),
    )
}
pub(super) fn suite(run: &mut Run, image: &str, _test_image: &str) -> Result<Value, String> {
    let disk = run.directory.join("motion01-user.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, "1920x1080")?;
    let result = drive(run, &record);
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = scene::capture_settled(run, &mut c, "motion01-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    let mut evidence = result?;
    seal_serial(&run.root, &mut record)?;
    evidence["session"] = record;
    evidence["method"] = json!(
        "Guest timestamps from the compositor's transition clock in DOOR_P12 ANIM records; WHPX-accelerated QEMU at 1920x1080; input by QMP"
    );
    evidence["scope"] = json!(
        "Timing on this virtual machine; not input-to-photon latency, frame pacing on hardware or a measured Windows 11 26H2 comparison"
    );
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn motion01_parses_records_and_judges_clear_violations() {
        let line = "DOOR_USER pid=5 DOOR_P12 ANIM k=20 t=81230 l=232 e=258 p=271 f=14 n=905";
        let a = parse(line).unwrap();
        assert_eq!(
            (a.kinds, a.start, a.rest, a.moving, a.frame),
            (MINIMIZE, 81_230, 258, 14, 905)
        );
        assert!(judge("minimize", &a, MINIMIZE_MS).is_ok());
        let early = Anim { rest: 120, ..a };
        assert!(
            judge("minimize", &early, MINIMIZE_MS)
                .unwrap_err()
                .contains("not time-based")
        );
        let late = Anim { rest: 900, ..a };
        assert!(judge("minimize", &late, MINIMIZE_MS).is_err());
        assert_eq!(
            parse("DOOR_P12 ANIM k=1 t=1 l=5 e=4 p=6 f=1 n=2"),
            None,
            "rest before last"
        );
        assert_eq!(
            parse("DOOR_P12 ANIM k=1 t=1 l=0 e=4 p=6 f=0 n=2"),
            None,
            "no frames"
        );
        assert_eq!(
            records(&format!("x\n{line}\nDOOR_P12 ANIM bad\n"), 0).len(),
            1
        );
    }
}
