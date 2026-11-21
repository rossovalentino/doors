//! Actual ordinary-boot workflows on a private disk, with independent raw-byte audit.
use super::*;
pub fn run(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("session-user.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut first = launch(run, image, &disk, false)?;
    let outcome = workflows(run, &first);
    if outcome.is_err() {
        let _ = quit(&first);
    }
    outcome?;
    shutdown(&run.root, &first)?;
    crate::evidence::seal_session_serial(&run.root, &mut first)?;
    let saved = audit(&disk, b"Hello from a persistent OS\nsecond line\n")?;
    let mut second = launch(run, image, &disk, false)?;
    let outcome = (|| {
        command(
            run,
            &second,
            "cat projects/renamed.txt",
            "Hello from a persistent OS\nsecond line\n",
        )?;
        command(
            run,
            &second,
            "./utility args reopened",
            "argv[2]=reopened\n",
        )?;
        command(run, &second, "ls projects", "renamed.txt")?;
        basic_command(
            run,
            &second,
            "tinybasic.exe projects/calculator.bas",
            "55\n1024\n",
        )?;
        command(run, &second, "bye", "")?;
        command(run, &second, "theme", "Theme 2")?;
        reboot(run, &second)?;
        command(
            run,
            &second,
            "cat projects/renamed.txt",
            "Hello from a persistent OS\nsecond line\n",
        )?;
        command(run, &second, "theme", "Theme 2")?;
        shutdown(&run.root, &second)
    })();
    if outcome.is_err() {
        let _ = quit(&second);
    }
    outcome?;
    crate::evidence::seal_session_serial(&run.root, &mut second)?;
    let reopened = audit(&disk, b"Hello from a persistent OS\nsecond line\n")?;
    let evidence = json!({"schema_version":1,"record_kind":"ordinary_session_suite","outcome":"passed","source_snapshot":run.build_inputs(),"image":image,"disk":relative(&run.root,&disk)?,"first":first,"second":second,"saved":saved,"reopened":reopened,"boot_test_token":false});
    write_json(
        &run.directory.join("ordinary-session-suite.json"),
        &evidence,
    )?;
    run.check("ordinary-session-suite", evidence);
    validate(&run.root, &json!({"checks":run.checks()}))?;
    Ok(())
}
const COMMANDS: &[(&str, &str)] = &[
    ("pwd", "/users/1001\n"),
    ("mkdir projects", ""),
    (
        "write projects/first.txt \"Hello from a persistent OS\"",
        "",
    ),
    ("append projects/first.txt \"second line\"", ""),
    (
        "cat projects/first.txt",
        "Hello from a persistent OS\nsecond line\n",
    ),
    ("cp projects/first.txt projects/copy.txt", ""),
    ("mv projects/copy.txt projects/renamed.txt", ""),
    ("rm projects/first.txt", ""),
    ("ls projects", "renamed.txt"),
    (
        "tools wc projects/renamed.txt",
        "2 7 39 projects/renamed.txt",
    ),
    ("tools args \"two words\"", "argv[2]=two words\n"),
    ("tools env", "PWD=/users/1001\n"),
    ("cp /system/bin/tools utility", ""),
    ("./utility args copied", "argv[2]=copied\n"),
    ("cat /system/bin/tools > binary-copy", ""),
    ("cd projects", ""),
    ("tools env", "PWD=/users/1001/projects\n"),
    ("tools wc renamed.txt", "2 7 39 renamed.txt"),
    ("cd ..", ""),
    ("cat missing-file.txt", "NotFound"),
    ("write /system/bin/forbidden nope", "Denied"),
    ("write malformed.exe invalid", ""),
    ("./malformed.exe", "Unsupported"),
    ("rm malformed.exe", ""),
    ("tools false", "exited 1"),
];
fn workflows(run: &mut Run, record: &Value) -> Result<(), String> {
    for &(line, expected) in COMMANDS {
        command(run, record, line, expected)?;
    }
    // Exceed the old 32-node ceiling using real namespace operations.
    for index in 0..36 {
        command(run, record, &format!("mkdir projects/d{index}"), "")?;
    }
    command(run, record, "ls projects", "d35")?;
    // Repeated launches exercise broker reaping and capability retirement.
    for _ in 0..12 {
        command(run, record, "tools true", "")?;
    }
    basic(run, record)?;
    type_line(record, "tools sleep 100000")?;
    thread::sleep(Duration::from_millis(500));
    command(run, record, "\x03", "exited -130")?;
    command(run, record, "theme 2", "Theme 2")?;
    Ok(())
}
fn basic(run: &mut Run, record: &Value) -> Result<(), String> {
    command(
        run,
        record,
        &format!(
            "tools repeat projects/big.bas \"rem {}\" 3300",
            "x".repeat(60)
        ),
        "",
    )?;
    for (line, expected) in [
        ("tinybasic.exe", "c> "),
        ("print 124\x083", "123\nc> "),
        ("10 a=0", "c> "),
        ("20 for i=1 to 10", "c> "),
        ("30 a=a+i", "c> "),
        ("40 next i", "c> "),
        ("50 print a", "c> "),
        ("60 print 2^10", "c> "),
        ("70 end", "c> "),
        ("run", "55\n1024\n"),
        ("save \"projects/calculator.bas\"", "c> "),
        ("new", "c> "),
        ("load \"projects/calculator.bas\"", "c> "),
        ("run", "55\n1024\n"),
        ("load \"projects/big.bas\"", "c> "),
        ("save \"projects/big-copy.bas\"", "c> "),
        ("new", "c> "),
    ] {
        basic_command(run, record, line, expected)?;
    }
    command(run, record, "bye", "")?;
    command(run, record, "write projects/after-save.txt healthy", "")?;
    command(run, record, "cat projects/calculator.bas", "50 print a")
}
fn basic_command(run: &mut Run, record: &Value, line: &str, expected: &str) -> Result<(), String> {
    let before = transcript(&run.root, record)?;
    type_line(record, line)?;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let after = transcript(&run.root, record)?;
        let delta = after
            .strip_prefix(&before)
            .ok_or("BASIC transcript changed")?;
        if delta.contains(expected) && delta.ends_with("c> ") {
            run.check(
                "tinybasic-guest",
                json!({"command":line,"expected":expected,"observed":delta}),
            );
            break;
        }
        if Instant::now() > deadline {
            return Err(format!(
                "Tiny Basic workflow {line:?} failed: {:?}",
                delta.chars().take(1024).collect::<String>()
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
fn command(run: &mut Run, record: &Value, line: &str, expected: &str) -> Result<(), String> {
    let before = transcript(&run.root, record)?;
    send_line(&run.root, record, line)?;
    let after = transcript(&run.root, record)?;
    let delta = after
        .strip_prefix(&before)
        .ok_or("terminal transcript changed")?;
    if !delta.contains(expected)
        || (expected.is_empty()
            && (delta.contains("Service(")
                || delta.contains("Transport(")
                || delta.contains("Command syntax:")))
    {
        return Err(format!(
            "Ordinary workflow {line:?}: expected {expected:?}, observed {delta:?}"
        ));
    }
    run.check(
        "ordinary-command",
        json!({"command":line,"expected":expected,"observed":delta}),
    );
    Ok(())
}
fn quit(record: &Value) -> Result<(), String> {
    let mut qmp = qmp::Client::connect(record["qmp_port"].as_u64().ok_or("session port")? as u16)?;
    if qmp.execute("query-uuid", json!({}))?["UUID"] != record["uuid"] {
        return Err("session identity changed".into());
    }
    qmp.execute("quit", json!({}))?;
    Ok(())
}
fn shutdown(root: &Path, record: &Value) -> Result<(), String> {
    send_line(root, record, "shutdown")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let serial = paths::existing(root, record["serial"].as_str().ok_or("serial path")?)?;
    loop {
        let text = fs::read_to_string(&serial).map_err(|e| e.to_string())?;
        if text.contains("KERNEL_PANIC") || text.contains("INIT FAIL") {
            return Err("ordinary shutdown failed".into());
        }
        if text.contains("CLEANUP PASS")
            && text.contains("leaked=0")
            && text.contains("reclaimed=8")
            && std::net::TcpStream::connect((
                "127.0.0.1",
                record["qmp_port"].as_u64().ok_or("session port")? as u16,
            ))
            .is_err()
        {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("ordinary power-off timeout".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}
fn reboot(run: &mut Run, record: &Value) -> Result<(), String> {
    let before = transcript(&run.root, record)?;
    send_line(&run.root, record, "reboot")?;
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let after = transcript(&run.root, record)?;
        let delta = after
            .strip_prefix(&before)
            .ok_or("reboot transcript changed")?;
        if delta.contains("Type help.") && delta.ends_with("/users/1001> ") {
            run.check(
                "ordinary-reboot",
                json!({"session_uuid":record["uuid"],"observed":delta}),
            );
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("ordinary reboot did not return to shell".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}
fn audit(disk: &Path, expected: &[u8]) -> Result<Value, String> {
    let bytes = fs::read(disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&bytes)?;
    if journal.selected().committed {
        return Err("ordinary shutdown left committed journal".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&bytes)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("missing ordinary catalog")?;
    let paths = catalog.paths()?;
    let node = paths
        .get("/users/1001/projects/renamed.txt")
        .ok_or("missing persisted user file")?;
    let file = &tree.files[&format!("obj{:016x}", node.id)];
    if file.bytes != expected {
        return Err("independent persisted user bytes mismatch".into());
    }
    let program = paths.get("/system/bin/tools").ok_or("missing tools")?;
    for copy in ["/users/1001/utility", "/users/1001/binary-copy"] {
        let node = paths.get(copy).ok_or("missing binary copy")?;
        if tree.files[&format!("obj{:016x}", node.id)].bytes
            != tree.files[&format!("obj{:016x}", program.id)].bytes
        {
            return Err("binary copy changed executable bytes".into());
        }
    }
    if !paths.contains_key("/users/1001/projects/d35") {
        return Err("production namespace tail not persisted".into());
    }
    let large = paths
        .get("/users/1001/projects/big-copy.bas")
        .ok_or("large BASIC save absent")?;
    let content = &tree.files[&format!("obj{:016x}", large.id)].bytes;
    if content.len() <= 200 * 1024
        || content.iter().filter(|b| **b == b'\n').count() != 3300
        || !content
            .windows(64)
            .any(|s| s == format!("rem {}", "x".repeat(60)).as_bytes())
    {
        return Err("large BASIC save byte/line audit failed".into());
    }
    Ok(
        json!({"disk_sha256":file_hash(disk)?,"journal":journal.summary(),"tree":tree.summary(),"user_file_bytes":file.bytes.len(),"catalog_schema":3,"paths":paths.keys().collect::<Vec<_>>()}),
    )
}
pub fn validate(root: &Path, report: &Value) -> Result<(), String> {
    let checks = report["checks"]
        .as_array()
        .ok_or("ordinary check inventory")?;
    let suites: Vec<_> = checks
        .iter()
        .filter(|c| c["id"] == "ordinary-session-suite" && c["outcome"] == "passed")
        .collect();
    if suites.len() != 1 {
        return Err("one ordinary session suite required".into());
    }
    let suite = &suites[0]["details"];
    if suite["outcome"] != "passed"
        || suite["boot_test_token"] != false
        || suite["first"]["uuid"] == suite["second"]["uuid"]
        || suite["first"]["disk"] != suite["second"]["disk"]
    {
        return Err("ordinary separate boot identity".into());
    }
    for key in ["first", "second"] {
        let record = &suite[key];
        let args = record["argv"].as_array().ok_or("ordinary argv")?;
        if args
            .iter()
            .any(|a| a == "-fw_cfg" || a.as_str().is_some_and(|a| a.contains("isa-debug-exit")))
        {
            return Err("ordinary suite used diagnostic selection/exit".into());
        }
        if let Some(retained) = record["serial_retained"].as_str()
            && file_hash(&paths::existing(root, retained)?)? != record["serial_sha256"]
        {
            return Err("retained ordinary serial changed".into());
        }
        let serial = paths::existing(root, record["serial"].as_str().ok_or("ordinary serial")?)?;
        if file_hash(&serial)? != record["serial_sha256"] {
            return Err("ordinary serial changed".into());
        }
        let text = fs::read_to_string(serial).map_err(|e| e.to_string())?;
        if !text.contains("CLEANUP PASS")
            || !text.contains("leaked=0")
            || !text.contains("reclaimed=8")
            || text.contains("KERNEL_PANIC")
        {
            return Err("ordinary cleanup evidence".into());
        }
    }
    let terminals = [
        transcript(root, &suite["first"])?,
        transcript(root, &suite["second"])?,
    ];
    for &(command, expected) in COMMANDS {
        if !checks.iter().any(|c| {
            c["id"] == "ordinary-command"
                && c["details"]["command"] == command
                && receipt(c, expected, &terminals)
        }) {
            return Err(format!("ordinary workflow missing: {command}"));
        }
    }
    for (command, expected) in [
        ("tinybasic.exe", "c> "),
        ("print 124\x083", "123\nc> "),
        ("save \"projects/calculator.bas\"", "c> "),
        ("load \"projects/calculator.bas\"", "c> "),
        ("save \"projects/big-copy.bas\"", "c> "),
    ] {
        if !checks.iter().any(|c| {
            c["id"] == "tinybasic-guest"
                && c["outcome"] == "passed"
                && c["details"]["command"] == command
                && receipt(c, expected, &terminals)
        }) {
            return Err(format!("BASIC workflow missing: {command}"));
        }
    }
    let disk = paths::existing(root, suite["disk"].as_str().ok_or("ordinary disk")?)?;
    let independent = audit(&disk, b"Hello from a persistent OS\nsecond line\n")?;
    if independent != suite["reopened"] {
        return Err("ordinary independent disk audit changed".into());
    }
    Ok(())
}
fn receipt(check: &Value, expected: &str, terminals: &[String]) -> bool {
    check["outcome"] == "passed"
        && check["details"]["expected"] == expected
        && check["details"]["observed"]
            .as_str()
            .is_some_and(|observed| {
                !observed.is_empty()
                    && observed.contains(expected)
                    && terminals.iter().any(|t| t.contains(observed))
                    && (!expected.is_empty()
                        || (!observed.contains("Service(") && !observed.contains("Transport(")))
            })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipts_reject_expected_or_observed_drift_from_actual_terminal() {
        let terminals = vec!["/users/1001> tools args\nargv[1]=args\n/users/1001> ".into()];
        let receipt_value = json!({"outcome":"passed","details":{"expected":"argv[1]=args\n","observed":"tools args\nargv[1]=args\n/users/1001> "}});
        assert!(receipt(&receipt_value, "argv[1]=args\n", &terminals));
        let mut changed = receipt_value.clone();
        changed["details"]["expected"] = json!("");
        assert!(!receipt(&changed, "argv[1]=args\n", &terminals));
        changed = receipt_value;
        changed["details"]["observed"] = json!("invented argv[1]=args\n");
        assert!(!receipt(&changed, "argv[1]=args\n", &terminals));
    }
}
