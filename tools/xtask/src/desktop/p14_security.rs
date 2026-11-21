//! SEC-02 trusted sign-in subset (`cargo xtask test desktop --suite
//! sec-02-trusted`, acceptance :4294) and the shared credential-gate driver
//! every desktop launch uses (P14-LOGON, ADR 0020).
//!
//! The runner boots a private disk: first-run enrollment of a test PIN, Notes
//! typing, Win+L, then (while locked) typed text and a wrong PIN that must
//! reach only the trusted gate, five wrong PINs that start the service-owned
//! throttle, the right PIN refused during the throttle, and finally the right
//! PIN. The saved Notes bytes on the raw disk prove that nothing typed while
//! locked reached the application, and the locked screendump proves the
//! application surface was not composed.
use super::{audit, click, connection, key, seal_serial, serial, shutdown, start_apps, type_line};
use crate::{
    evidence::{Run, file_hash, relative},
    qmp, session,
};
use serde_json::{Value, json};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

/// Test-only PIN for private, disposable test volumes created by the runners.
/// It is never typed into the owner's volume (`out/session/user.raw`): the
/// interactive desktop leaves sign-in to the owner.
pub const TEST_PIN: &str = "2468";
/// A wrong PIN for negative cases (never equal to `TEST_PIN`).
const WRONG_PIN: &str = "1357";

/// The QMP keys that answer the gate the guest is showing, or `None` while no
/// usable gate surface has been reported yet. An unavailable gate (the session
/// service is not up yet) is retried by the guest every second, so it means
/// "wait"; there is no passwordless path, and a gate that never becomes
/// usable ends in the launch readiness timeout.
pub fn sign_in_keys(serial: &str) -> Option<Result<Vec<String>, String>> {
    // Only complete lines: the serial file is read while the guest writes,
    // and a half-written gate line would lack its `kind=` field.
    let complete = serial.rfind('\n').map_or("", |end| &serial[..end]);
    let line = complete
        .lines()
        .rev()
        .find(|line| line.contains("DOOR_P14 SIGNIN surface="))?;
    let surface = line
        .split_once("surface=")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .unwrap_or_default();
    let pin: Vec<String> = TEST_PIN.chars().map(String::from).collect();
    let enter = || "ret".to_owned();
    Some(match surface {
        "enroll" => {
            if !line.contains("kind=pin") {
                return Some(Err("first-run gate does not offer the default PIN".into()));
            }
            let mut keys = pin.clone();
            keys.push(enter());
            keys.extend(pin);
            keys.push(enter());
            Ok(keys)
        }
        "signin" => {
            let mut keys = pin;
            keys.push(enter());
            Ok(keys)
        }
        "unavailable" => return None,
        other => Err(format!(
            "trusted sign-in gate reported surface={other}; no passwordless fallback exists"
        )),
    })
}

fn wait_after(
    root: &std::path::Path,
    record: &Value,
    offset: usize,
    needle: &str,
    timeout: Duration,
) -> Result<String, String> {
    wait_after_any(root, record, offset, &[needle], timeout)
}
/// `wait_after` for the first line containing any of `needles`.
fn wait_after_any(
    root: &std::path::Path,
    record: &Value,
    offset: usize,
    needles: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let text = serial(root, record)?;
        if let Some(found) = text.get(offset..).and_then(|tail| {
            tail.lines()
                .find(|line| needles.iter().any(|needle| line.contains(needle)))
                .map(str::to_owned)
        }) {
            return Ok(found);
        }
        if text.contains("KERNEL_PANIC") || text.contains("INIT FAIL") {
            return Err("desktop guest failed; inspect serial".into());
        }
        if Instant::now() > deadline {
            return Err(format!(
                "SEC-02: guest did not report {}",
                needles.join(" or ")
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
fn type_pin(client: &mut qmp::Client, pin: &str) -> Result<(), String> {
    for ch in pin.chars() {
        key(client, &[ch.to_string().as_str()])?;
    }
    key(client, &["ret"])
}
fn screendump(
    run: &Run,
    client: &mut qmp::Client,
    name: &str,
) -> Result<std::path::PathBuf, String> {
    let path = run.directory.join(format!("sec02-{name}.ppm"));
    client.execute(
        "screendump",
        json!({"filename":path.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/")}),
    )?;
    thread::sleep(Duration::from_millis(300));
    Ok(path)
}
/// Mean absolute RGB difference over the centre half of two captures.
fn centre_difference(a: &std::path::Path, b: &std::path::Path) -> Result<f64, String> {
    let (a, b) = (
        crate::fidelity::Image::read(a)?,
        crate::fidelity::Image::read(b)?,
    );
    if (a.width, a.height) != (b.width, b.height) {
        return Err("SEC-02: capture sizes differ".into());
    }
    let (x0, y0, x1, y1) = (a.width / 4, a.height / 4, a.width * 3 / 4, a.height * 3 / 4);
    let mut total = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let (p, q) = (a.pixel(x, y), b.pixel(x, y));
            total += (0..3).map(|i| u64::from(p[i].abs_diff(q[i]))).sum::<u64>();
        }
    }
    Ok(total as f64 / ((x1 - x0) * (y1 - y0) * 3) as f64)
}

pub fn sec_02_trusted_suite(
    run: &mut Run,
    image: &str,
    _test_image: &str,
) -> Result<Value, String> {
    let disk = run.directory.join("sec02-private.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    // The launch drives first-run enrollment through `sign_in_keys`.
    let mut record = session::launch_desktop(run, image, &disk, false)?;
    let result = drive(run, &record);
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = screendump(run, &mut c, "failure");
        let _ = c.execute("quit", json!({}));
    }
    let mut evidence = result?;
    seal_serial(&run.root, &mut record)?;
    // Nothing typed while locked (letters, wrong PINs, the right PIN) reached
    // Notes: the document holds exactly what was typed while unlocked.
    let saved = audit(&disk, b"before lock\n after unlock\n")?;
    evidence["persisted_document"] = saved;
    evidence["disk_sha256"] = json!(file_hash(&disk)?);
    evidence["reference_comparison"] = json!("not_applicable");
    Ok(evidence)
}

fn drive(run: &mut Run, record: &Value) -> Result<Value, String> {
    let root = run.root.clone();
    let text = serial(&root, record)?;
    // Enrollment happened through the trusted gate before any session work.
    let enrolled = text
        .find("DOOR_P14 SIGNIN surface=enroll kind=pin purpose=signin")
        .ok_or("SEC-02: first run did not show the enrollment gate")?;
    let accepted = text
        .find("DOOR_P14 SIGNIN accepted purpose=signin")
        .ok_or("SEC-02: enrollment was not accepted")?;
    let credential = text
        .lines()
        .find(|l| l.contains("DOOR_P14 CREDENTIAL enrolled persisted="))
        .ok_or("SEC-02: the session service did not report the enrolled record")?
        .to_owned();
    let ready = text
        .find("DOOR_P11 READY")
        .ok_or("SEC-02: desktop not ready")?;
    if !(enrolled < accepted && accepted < ready) {
        return Err("SEC-02: the session became ready before the credential gate accepted".into());
    }
    let mut c = connection(record)?;
    let mut pointer = (512, 384);
    start_apps(&root, record, &mut c)?;
    click(&mut c, &mut pointer, 850, 390)?;
    type_line(&mut c, "before lock")?;
    key(&mut c, &["ctrl", "s"])?;
    super::wait_serial(
        &root,
        record,
        "DOOR_P11 NOTES SAVED",
        Duration::from_secs(10),
    )?;
    let unlocked = screendump(run, &mut c, "unlocked")?;

    // Win+L: the gate re-arms for unlock.
    let offset = serial(&root, record)?.len();
    key(&mut c, &["meta_l", "l"])?;
    wait_after(
        &root,
        record,
        offset,
        "DOOR_P12 LOCKED",
        Duration::from_secs(10),
    )?;
    let surface = wait_after(
        &root,
        record,
        offset,
        "DOOR_P14 SIGNIN surface=signin kind=pin purpose=unlock",
        Duration::from_secs(10),
    )?;
    thread::sleep(Duration::from_millis(800));
    let locked = screendump(run, &mut c, "locked")?;
    let difference = centre_difference(&unlocked, &locked)?;
    if difference < 8.0 {
        return Err(format!(
            "SEC-02: the locked screen still resembles the application desktop (mean difference {difference:.1})"
        ));
    }

    // Injection attempts while locked: letters (dropped by the PIN box), a
    // bare Enter, then a wrong PIN, which the service rejects.
    type_line(&mut c, "inject")?;
    let offset = serial(&root, record)?.len();
    type_pin(&mut c, WRONG_PIN)?;
    let first = wait_after(
        &root,
        record,
        offset,
        "DOOR_P14 SIGNIN rejected failures=1 ",
        Duration::from_secs(20),
    )?;
    // Four more wrong PINs start the service-owned throttle.
    for failures in 2..=5 {
        let offset = serial(&root, record)?.len();
        type_pin(&mut c, WRONG_PIN)?;
        wait_after(
            &root,
            record,
            offset,
            &format!("DOOR_P14 SIGNIN rejected failures={failures} "),
            Duration::from_secs(20),
        )?;
    }
    let throttle = wait_after(
        &root,
        record,
        0,
        "DOOR_P14 SIGNIN rejected failures=5 retry_ms=30000",
        Duration::from_secs(1),
    )?;
    // The right PIN during the throttle is refused without unlocking.
    let offset = serial(&root, record)?.len();
    type_pin(&mut c, TEST_PIN)?;
    let refused = wait_after(
        &root,
        record,
        offset,
        "DOOR_P14 SIGNIN throttled",
        Duration::from_secs(10),
    )?;
    let throttled_at = Instant::now();
    if serial(&root, record)?[offset..].contains("DOOR_P12 UNLOCKED") {
        return Err("SEC-02: the session unlocked during the throttle".into());
    }
    let throttled = screendump(run, &mut c, "throttled")?;
    // Wait out the 30 s delay, then the right PIN unlocks. Guest time can run
    // behind the host under parallel TCG load; while the gate still refuses
    // the PIN it reports the remaining delay, so wait that out and retry
    // (bounded). An unlock before the gate accepts still fails.
    let wait = Duration::from_secs(31).saturating_sub(throttled_at.elapsed());
    thread::sleep(wait);
    let mut offset = serial(&root, record)?.len();
    for attempt in 0..4 {
        type_pin(&mut c, TEST_PIN)?;
        let line = wait_after_any(
            &root,
            record,
            offset,
            &[
                "DOOR_P14 SIGNIN accepted purpose=unlock",
                "DOOR_P14 SIGNIN throttled",
            ],
            Duration::from_secs(20),
        )?;
        if line.contains("accepted") {
            break;
        }
        if attempt == 3 || serial(&root, record)?[offset..].contains("DOOR_P12 UNLOCKED") {
            return Err(format!(
                "SEC-02: the right PIN stayed refused past the delay: {line}"
            ));
        }
        let remaining = line
            .split("retry_ms=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1000)
            .min(30_000);
        thread::sleep(Duration::from_millis(remaining * 2 + 500));
        offset = serial(&root, record)?.len();
    }
    wait_after(
        &root,
        record,
        offset,
        "DOOR_P12 UNLOCKED",
        Duration::from_secs(10),
    )?;
    let unlocked_again = screendump(run, &mut c, "unlocked-again")?;
    click(&mut c, &mut pointer, 850, 390)?;
    type_line(&mut c, " after unlock")?;
    // The offset precedes Ctrl+S: a fast save reports before a later read.
    let offset = serial(&root, record)?.len();
    key(&mut c, &["ctrl", "s"])?;
    wait_after(
        &root,
        record,
        offset,
        "DOOR_P11 NOTES SAVED",
        Duration::from_secs(10),
    )?;
    let full = serial(&root, record)?;
    if full
        .lines()
        .filter(|line| line.contains("DOOR_P14"))
        .any(|line| line.contains(TEST_PIN) || line.contains(WRONG_PIN))
    {
        return Err("SEC-02: a typed PIN appeared in the serial evidence".into());
    }
    shutdown(&root, record, &mut c, &mut pointer)?;
    let shot = |path: &std::path::Path| -> Result<Value, String> {
        Ok(json!({"path":relative(&run.root,path)?,"sha256":file_hash(path)?}))
    };
    Ok(json!({
        "enrollment": {"surface":"enroll kind=pin","service":credential,
            "accepted_before_session_ready": true},
        "lock": {"gesture":"Win+L (QMP)","surface":surface,
            "locked_centre_mean_difference":difference,
            "captures":{"unlocked":shot(&unlocked)?,"locked":shot(&locked)?,
                "throttled":shot(&throttled)?,"unlocked_again":shot(&unlocked_again)?}},
        "injection": {"typed_while_locked":["inject","wrong PIN x5","right PIN during throttle"],
            "reached_application": false},
        "rejection": first,
        "throttle": {"after_failures":5,"record":throttle,"refused":refused,"waited_ms":31000},
        "unlock": "right PIN after the delay",
        "secret_in_serial": false,
        "scope": "SEC-02 lock subset: trusted gate, service-verified Argon2id credential, throttling, input isolation and lock-screen composition; trusted elevation prompts and capture APIs are not part of P14"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_driver_answers_each_surface_and_never_falls_back() {
        assert!(sign_in_keys("DOOR_P12 SIGNIN DISPLAY READY\n").is_none());
        let enroll = "DOOR_P14 SIGNIN surface=enroll kind=pin purpose=signin persisted=0\n";
        assert_eq!(
            sign_in_keys(enroll).unwrap().unwrap(),
            ["2", "4", "6", "8", "ret", "2", "4", "6", "8", "ret"]
        );
        let signin =
            format!("{enroll}DOOR_P14 SIGNIN surface=signin kind=pin purpose=signin persisted=1\n");
        assert_eq!(
            sign_in_keys(&signin).unwrap().unwrap(),
            ["2", "4", "6", "8", "ret"]
        );
        // Unavailable means the session service is not up yet: keep waiting.
        let waiting = "DOOR_P14 SIGNIN surface=unavailable purpose=signin\n";
        assert!(sign_in_keys(waiting).is_none());
        assert!(sign_in_keys(&format!("{waiting}{enroll}")).unwrap().is_ok());
        // A line still being written is not answered yet.
        assert!(sign_in_keys("DOOR_P14 SIGNIN surface=enroll").is_none());
        for bad in [
            "DOOR_P14 SIGNIN surface=enroll kind=password purpose=signin persisted=0\n",
            "DOOR_P14 SIGNIN surface=other purpose=signin\n",
        ] {
            assert!(sign_in_keys(bad).unwrap().is_err(), "{bad}");
        }
        assert_ne!(TEST_PIN, WRONG_PIN);
    }
}
