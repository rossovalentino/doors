use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub struct Run {
    pub root: PathBuf,
    pub directory: PathBuf,
    pub id: String,
    pub certified_phase: Option<String>,
    pub output_layout: crate::layout::Layout,
    commands: Vec<Value>,
    /// Command numbers of eery ledger writing into `directory` (the run and
    /// its forked ledgers), so `command-NN` logs never collide.
    command_counter: Arc<AtomicUsize>,
    /// A forked ledger (`fork_ledger`) records only; its parent finishes.
    forked: bool,
    checks: Vec<Value>,
    guest_result: Option<Value>,
    guest_suites: Vec<Value>,
    sessions: Vec<Value>,
    reference_comparison: Value,
    start_snapshot: Value,
    /// Elapsed time per stage, build, guest task and suite (`record_timing`).
    timings: Vec<Value>,
    /// How guest work was scheduled (`parallel::concurrency`), when it was.
    scheduling: Option<Value>,
}

/// A process-unique, increasing nanosecond stamp for run and guest session
/// identities: concurrent workers of one process never share one.
pub(crate) fn unique_stamp() -> Result<u128, String> {
    static LAST: Mutex<u128> = Mutex::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let mut last = LAST.lock().unwrap_or_else(PoisonError::into_inner);
    *last = now.max(*last + 1);
    Ok(*last)
}

/// Runs `stage` and records its elapsed time and outcome on `run`.
pub fn timed<T>(
    run: &mut Run,
    kind: &str,
    name: &str,
    stage: impl FnOnce(&mut Run) -> Result<T, String>,
) -> Result<T, String> {
    let started = Instant::now();
    let result = stage(run);
    run.record_timing(
        kind,
        name,
        started.elapsed(),
        if result.is_ok() { "passed" } else { "failed" },
    );
    result
}

pub struct Observation {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub tool_error: bool,
    pub output_limit_exceeded: bool,
    pub cleanup_failed: bool,
}

impl Run {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        let start_snapshot = snapshot(&root)?;
        let runs = root.join("reports/runs");
        fs::create_dir_all(&runs).map_err(|e| e.to_string())?;
        // Each run claims its own directory exclusively: concurrent child runs
        // of one process get distinct stamps and never share a ledger.
        let (id, directory) = loop {
            let id = format!("host-{}-{}", unique_stamp()?, std::process::id());
            let directory = runs.join(&id);
            match fs::create_dir(&directory) {
                Ok(()) => break (id, directory),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        };
        Ok(Self {
            root,
            directory,
            id,
            certified_phase: None,
            output_layout: crate::layout::Layout::Legacy,
            commands: Vec::new(),
            command_counter: Arc::new(AtomicUsize::new(0)),
            forked: false,
            checks: Vec::new(),
            guest_result: None,
            guest_suites: Vec::new(),
            sessions: Vec::new(),
            reference_comparison: json!("not_run"),
            start_snapshot,
            timings: Vec::new(),
            scheduling: None,
        })
    }

    /// A private ledger for one worker of this run: same identity, directory,
    /// layout and source snapshot, its own command/check/session records and
    /// shared command numbering. The worker's files and guest sessions are
    /// this run's; `absorb` appends its records here, in the caller's order.
    pub fn fork_ledger(&self) -> Self {
        Self {
            root: self.root.clone(),
            directory: self.directory.clone(),
            id: self.id.clone(),
            certified_phase: self.certified_phase.clone(),
            output_layout: self.output_layout,
            commands: Vec::new(),
            command_counter: Arc::clone(&self.command_counter),
            forked: true,
            checks: Vec::new(),
            guest_result: None,
            guest_suites: Vec::new(),
            sessions: Vec::new(),
            reference_comparison: json!("not_run"),
            start_snapshot: self.start_snapshot.clone(),
            timings: Vec::new(),
            scheduling: None,
        }
    }

    /// Appends every record of a ledger forked from this run.
    pub fn absorb(&mut self, ledger: Self) {
        debug_assert!(ledger.forked && Arc::ptr_eq(&ledger.command_counter, &self.command_counter));
        self.commands.extend(ledger.commands);
        self.checks.extend(ledger.checks);
        self.sessions.extend(ledger.sessions);
        self.guest_suites.extend(ledger.guest_suites);
        self.timings.extend(ledger.timings);
        if ledger.guest_result.is_some() {
            self.guest_result = ledger.guest_result;
        }
        if ledger.reference_comparison != "not_run" {
            self.reference_comparison = ledger.reference_comparison;
        }
    }

    fn next_command_id(&self) -> String {
        format!(
            "command-{:02}",
            self.command_counter.fetch_add(1, Ordering::SeqCst) + 1
        )
    }

    /// Records the elapsed time of one stage, build, guest task or suite.
    pub fn record_timing(&mut self, kind: &str, name: &str, elapsed: Duration, outcome: &str) {
        self.timings.push(json!({"kind":kind,"name":name,
            "elapsed_ms":elapsed.as_millis(),"outcome":outcome}));
    }

    /// How guest work was scheduled (worker count and its inputs).
    pub fn record_scheduling(&mut self, record: Value) {
        self.scheduling = Some(record);
    }

    /// The `top` longest recorded timings and host commands, longest first.
    pub fn timing_summary(&self, top: usize) -> String {
        let mut rows: Vec<(u128, String, String, String)> = self
            .timings
            .iter()
            .map(|t| {
                (
                    t["elapsed_ms"].as_u64().map_or(0, u128::from),
                    t["kind"].as_str().unwrap_or("").to_owned(),
                    t["name"].as_str().unwrap_or("").to_owned(),
                    t["outcome"].as_str().unwrap_or("").to_owned(),
                )
            })
            .chain(self.commands.iter().map(|c| {
                let argv: Vec<&str> = c["argv"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                let program = argv
                    .first()
                    .and_then(|p| Path::new(p).file_name())
                    .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
                let mut name = std::iter::once(program.as_str())
                    .chain(argv.iter().skip(1).copied())
                    .collect::<Vec<_>>()
                    .join(" ");
                if name.chars().count() > 110 {
                    name = name.chars().take(107).collect::<String>() + "...";
                }
                (
                    c["duration_ms"].as_u64().map_or(0, u128::from),
                    "command".to_owned(),
                    name,
                    c["outcome"].as_str().unwrap_or("").to_owned(),
                )
            }))
            .collect();
        rows.sort_by(|a, b| b.0.cmp(&a.0));
        let mut text = format!(
            "Timing summary: {} longest of {} recorded steps and commands",
            top.min(rows.len()),
            rows.len()
        );
        for (ms, kind, name, outcome) in rows.into_iter().take(top) {
            text.push_str(&format!(
                "\n  {:>9.1} s  {kind:<17} {name} [{outcome}]",
                ms as f64 / 1000.0
            ));
        }
        text
    }

    pub fn generated_path(&self, legacy: &str) -> Result<String, String> {
        self.output_layout.path(&self.id, legacy)
    }

    pub fn generated_directory(&self, legacy: &str) -> Result<PathBuf, String> {
        crate::paths::fresh_directory(&self.root, &self.generated_path(legacy)?)
    }

    /// Development builds only (`DOOR_XTASK_DEV_CACHE=1`): a persistent
    /// cargo target directory under ignored `target/xtask-dev/<name>`, so a
    /// rebuild compiles only what changed. The run records that it used
    /// one, and such a run never qualifies a gate (`verify` refuses it).
    pub fn dev_cache(&self, name: &str) -> Result<Option<PathBuf>, String> {
        if !dev_cache_enabled() {
            return Ok(None);
        }
        let directory = self.root.join("target/xtask-dev").join(name);
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        write_json(
            &self.directory.join("dev-cache.json"),
            &json!({"dev_target_cache":"target/xtask-dev","qualifying":false}),
        )?;
        Ok(Some(directory))
    }

    pub fn required(&mut self, program: &str, args: &[&str]) -> Result<Observation, String> {
        let observed = self.command(program, args, true, required_timeout(program, args))?;
        if !observed.success {
            return Err(format!(
                "{program} {args:?} failed; see {}",
                self.directory.display()
            ));
        }
        Ok(observed)
    }

    pub fn optional(&mut self, program: &str, args: &[&str]) -> Result<Observation, String> {
        self.command(program, args, false, Duration::from_secs(30))
    }

    fn command(
        &mut self,
        program: &str,
        args: &[&str],
        required: bool,
        timeout: Duration,
    ) -> Result<Observation, String> {
        self.bounded(program, args, timeout, &[], required)
    }

    pub fn bounded(
        &mut self,
        program: &str,
        args: &[&str],
        timeout: Duration,
        environment: &[(&str, &str)],
        required: bool,
    ) -> Result<Observation, String> {
        self.execute(
            program,
            args,
            ProcessPolicy {
                timeout,
                environment,
                required,
                watched: &[],
                expected_exit: 0,
                directory: None,
            },
        )
    }

    pub fn watched(
        &mut self,
        program: &str,
        args: &[&str],
        timeout: Duration,
        expected_exit: i32,
        outputs: &[(&Path, u64)],
    ) -> Result<Observation, String> {
        self.execute(
            program,
            args,
            ProcessPolicy {
                timeout,
                environment: &[],
                required: false,
                watched: outputs,
                expected_exit,
                directory: None,
            },
        )
    }
    /// Run an ordinary fixture in a private output directory while retaining the
    /// exact working directory, command and raw streams in the normal ledger.
    pub fn watched_in(
        &mut self,
        directory: &Path,
        program: &str,
        args: &[&str],
        timeout: Duration,
        expected_exit: i32,
    ) -> Result<Observation, String> {
        self.execute(
            program,
            args,
            ProcessPolicy {
                directory: Some(directory),
                timeout,
                environment: &[],
                required: false,
                expected_exit,
                watched: &[],
            },
        )
    }
    fn execute(
        &mut self,
        program: &str,
        args: &[&str],
        policy: ProcessPolicy<'_>,
    ) -> Result<Observation, String> {
        let ProcessPolicy {
            timeout,
            environment,
            required,
            watched,
            expected_exit,
            directory,
        } = policy;
        let directory = directory
            .unwrap_or(&self.root)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let root = self.root.canonicalize().map_err(|e| e.to_string())?;
        if !directory.starts_with(&root) || !directory.is_dir() {
            return Err("command working directory must remain within the workspace".into());
        }
        let id = self.next_command_id();
        let stdout_path = self.directory.join(format!("{id}.stdout.log"));
        let stderr_path = self.directory.join(format!("{id}.stderr.log"));
        let stdout = File::create(&stdout_path).map_err(|e| e.to_string())?;
        let stderr = File::create(&stderr_path).map_err(|e| e.to_string())?;
        let started = Instant::now();
        let mut exit_code = None;
        let mut timed_out = false;
        let mut output_limit_exceeded = false;
        let mut spawn_error = None;
        let mut cleanup_error = None;
        let mut output_budget_error = None;
        let mut budget_checked = Instant::now();
        // A child starts in the ordinary drive-letter form of the canonical
        // directory, not the verbatim `\\?\` form: the Windows shell (common
        // dialogs) does not parse verbatim current directories.
        let spawn_directory = directory
            .to_str()
            .and_then(|path| path.strip_prefix(r"\\?\"))
            .filter(|path| path.as_bytes().get(1) == Some(&b':'))
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.clone());
        // Argument arrays and files avoid shell interpolation and pipe deadlocks.
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(environment.iter().copied())
            .current_dir(&spawn_directory)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        match command.spawn() {
            Ok(mut child) => loop {
                if budget_checked.elapsed() >= Duration::from_secs(1) {
                    output_budget_error = crate::cache::writer_headroom(&self.root).err();
                    budget_checked = Instant::now();
                }
                match child.try_wait() {
                    Ok(Some(status)) => {
                        exit_code = status.code();
                        break;
                    }
                    Ok(None)
                        if started.elapsed() < timeout
                            && !log_limit(&stdout_path, &stderr_path)
                            && output_budget_error.is_none()
                            && !watched_limit(watched) =>
                    {
                        thread::sleep(Duration::from_millis(20))
                    }
                    Ok(None) => {
                        timed_out = started.elapsed() >= timeout;
                        output_limit_exceeded =
                            log_limit(&stdout_path, &stderr_path) || watched_limit(watched);
                        // P00 commands are trusted host tools. On Windows also
                        // terminate their descendants, then reap the direct child.
                        #[cfg(windows)]
                        {
                            let result = Command::new("taskkill.exe")
                                .args(["/PID", &child.id().to_string(), "/T", "/F"])
                                .stdin(Stdio::null())
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .status();
                            if !result.is_ok_and(|status| status.success()) {
                                cleanup_error = Some(
                                    "taskkill failed; direct child kill attempted".to_string(),
                                );
                            }
                        }
                        if let Err(error) = child.kill() {
                            cleanup_error.get_or_insert(error.to_string());
                        }
                        match child.wait() {
                            Ok(status) => exit_code = status.code(),
                            Err(error) => {
                                cleanup_error.get_or_insert(error.to_string());
                            }
                        }
                        break;
                    }
                    Err(error) => {
                        cleanup_error = Some(error.to_string());
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            },
            Err(error) => spawn_error = Some(error.to_string()),
        }
        output_limit_exceeded |= log_limit(&stdout_path, &stderr_path) || watched_limit(watched);
        for path in [&stdout_path, &stderr_path] {
            if fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_LOG_BYTES {
                File::options()
                    .write(true)
                    .open(path)
                    .and_then(|file| file.set_len(MAX_LOG_BYTES))
                    .map_err(|e| e.to_string())?;
            }
        }
        for (path, limit) in watched {
            if fs::metadata(path).is_ok_and(|metadata| metadata.len() > *limit) {
                File::options()
                    .write(true)
                    .open(path)
                    .and_then(|file| file.set_len(*limit))
                    .map_err(|e| e.to_string())?;
            }
        }
        let success = exit_code == Some(expected_exit)
            && !timed_out
            && !output_limit_exceeded
            && spawn_error.is_none()
            && cleanup_error.is_none();
        let success = success && output_budget_error.is_none();
        let outcome = if timed_out {
            "timeout"
        } else if output_budget_error.is_some() {
            "output_budget"
        } else if output_limit_exceeded {
            "output_limit"
        } else if spawn_error.is_some() {
            "tool_error"
        } else if success {
            "passed"
        } else {
            "failed"
        };
        let stdout = String::from_utf8_lossy(&fs::read(&stdout_path).map_err(|e| e.to_string())?)
            .into_owned();
        let stderr = String::from_utf8_lossy(&fs::read(&stderr_path).map_err(|e| e.to_string())?)
            .into_owned();
        let command = json!({
            "id": id, "argv": std::iter::once(program).chain(args.iter().copied()).collect::<Vec<_>>(),
            "cwd": directory, "resolved_executable": resolve_tool(program),
            "required": required, "expected_exit_code": expected_exit, "exit_code": exit_code,
            "timed_out": timed_out, "timeout_ms": timeout.as_millis(),
            "duration_ms": started.elapsed().as_millis(), "spawn_error": spawn_error,
            "cleanup_error": cleanup_error, "outcome": outcome,
            "output_budget_error":output_budget_error,
            "environment_overrides":environment,"output_limit_bytes_per_stream":MAX_LOG_BYTES,"output_limit_exceeded":output_limit_exceeded,
            "watched_output_limits":watched.iter().map(|(path,limit)|json!({"path":path,"limit_bytes":limit})).collect::<Vec<_>>(),
            "stdout": {"path": relative(&self.root, &stdout_path)?, "sha256": file_hash(&stdout_path)?},
            "stderr": {"path": relative(&self.root, &stderr_path)?, "sha256": file_hash(&stderr_path)?}
        });
        self.commands.push(command);
        println!(
            "{}[{outcome}] {program} {}",
            crate::parallel::console_prefix(),
            args.join(" ")
        );
        if required && !success {
            eprint!("{stdout}{stderr}");
        }
        Ok(Observation {
            success,
            stdout,
            stderr,
            exit_code,
            timed_out,
            tool_error: spawn_error.is_some(),
            output_limit_exceeded,
            cleanup_failed: cleanup_error.is_some(),
        })
    }

    pub fn commands(&self) -> &[Value] {
        &self.commands
    }

    /// Drives an interactive child through checked polling. Only an explicit
    /// owner handoff permits a live process to outlast this evidence run.
    pub fn controlled<T>(
        &mut self,
        program: &str,
        args: &[String],
        outputs: &[(&Path, u64)],
        controller: impl FnOnce(&mut ControlledProcess<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        self.controlled_expected(program, args, outputs, 0, controller)
    }

    /// Interactive guests can complete via isa-debug-exit. The explicit code
    /// remains part of process evidence, with unchanged timeout/cleanup policy.
    pub fn controlled_expected<T>(
        &mut self,
        program: &str,
        args: &[String],
        outputs: &[(&Path, u64)],
        expected_exit: i32,
        controller: impl FnOnce(&mut ControlledProcess<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        self.controlled_expected_env(program, args, outputs, expected_exit, &[], controller)
    }

    /// Child-only structured overrides, sealed alongside the actual process.
    /// Never mutate the orchestrator's environment to configure a viewer.
    pub fn controlled_expected_env<T>(
        &mut self,
        program: &str,
        args: &[String],
        outputs: &[(&Path, u64)],
        expected_exit: i32,
        environment: &[(&str, &str)],
        controller: impl FnOnce(&mut ControlledProcess<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let id = self.next_command_id();
        let stdout_path = self.directory.join(format!("{id}.stdout.log"));
        let stderr_path = self.directory.join(format!("{id}.stderr.log"));
        let started = Instant::now();
        let spawned = Command::new(program)
            .args(args)
            .envs(environment.iter().copied())
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(File::create(&stdout_path).map_err(|e| e.to_string())?)
            .stderr(File::create(&stderr_path).map_err(|e| e.to_string())?)
            .spawn();
        let spawn_error = spawned.as_ref().err().map(ToString::to_string);
        let mut process = ControlledProcess {
            child: spawned.ok(),
            started,
            stdout: &stdout_path,
            stderr: &stderr_path,
            watched: outputs,
            exit: None,
            handed_off: false,
            timed_out: false,
            output_limit_exceeded: false,
            cleanup_error: None,
            output_root: &self.root,
            budget_checked: Instant::now(),
            output_budget_error: None,
        };
        let pid = process.child.as_ref().map(Child::id);
        let result = if let Some(error) = &spawn_error {
            Err(error.clone())
        } else {
            controller(&mut process)
        };
        if !process.handed_off && process.exit.is_none() {
            process.terminate();
        }
        process.output_limit_exceeded |=
            log_limit(&stdout_path, &stderr_path) || watched_limit(outputs);
        for (path, limit) in [
            (stdout_path.as_path(), MAX_LOG_BYTES),
            (stderr_path.as_path(), MAX_LOG_BYTES),
        ]
        .into_iter()
        .chain(outputs.iter().copied())
        {
            if fs::metadata(path).is_ok_and(|m| m.len() > limit) {
                File::options()
                    .write(true)
                    .open(path)
                    .and_then(|f| f.set_len(limit))
                    .map_err(|e| e.to_string())?;
            }
        }
        let success = result.is_ok()
            && spawn_error.is_none()
            && process.cleanup_error.is_none()
            && !process.timed_out
            && !process.output_limit_exceeded
            && process.output_budget_error.is_none()
            && (process.handed_off || process.exit.and_then(|s| s.code()) == Some(expected_exit));
        self.commands.push(json!({"id":id,"argv":std::iter::once(program).chain(args.iter().map(String::as_str)).collect::<Vec<_>>(),
            "cwd":self.root,"resolved_executable":resolve_tool(program),"pid":pid,
            "environment_overrides":environment,
            "required":!process.handed_off,"expected_exit_code":expected_exit,"exit_code":process.exit.and_then(|s|s.code()),
            "lifecycle":if process.handed_off{"running"}else{"completed"},
            "timed_out":process.timed_out,"timeout_ms":90000,"duration_ms":started.elapsed().as_millis(),
            "spawn_error":spawn_error,"cleanup_error":process.cleanup_error,"controller_error":result.as_ref().err(),
            "output_budget_error":process.output_budget_error,
            "outcome":if !success{"failed"}else if process.handed_off{"running"}else{"passed"},
            "output_limit_bytes_per_stream":MAX_LOG_BYTES,"output_limit_exceeded":process.output_limit_exceeded,
            "watched_output_limits":outputs.iter().map(|(path,limit)|json!({"path":path,"limit_bytes":limit})).collect::<Vec<_>>(),
            "stdout":{"path":relative(&self.root,&stdout_path)?,"sha256":file_hash(&stdout_path)?},
            "stderr":{"path":relative(&self.root,&stderr_path)?,"sha256":file_hash(&stderr_path)?}}));
        if !success {
            return Err(result
                .err()
                .unwrap_or_else(|| "interactive process did not complete cleanly".into()));
        }
        result
    }
    pub fn checks(&self) -> &[Value] {
        &self.checks
    }

    pub fn check(&mut self, id: &str, details: Value) {
        self.checks
            .push(json!({"id":id,"outcome":"passed","details":details}));
    }
    pub fn build_inputs(&self) -> Value {
        self.start_snapshot.clone()
    }
    pub fn record_guest(&mut self, record: &Value) -> Result<(), String> {
        if !record["reference_comparison"].is_null() {
            self.reference_comparison = record["reference_comparison"].clone();
        }
        let path = self.directory.join("guest-run.json");
        write_json(&path, record)?;
        self.guest_result = Some(
            json!({"path":relative(&self.root,&path)?,"sha256":file_hash(&path)?,
            "outcome":record["outcome"],"verification_state":record["verification_state"]}),
        );
        Ok(())
    }
    /// A launched guest session: its serial log, staged image record and
    /// screenshots are retained at `finish` even when no check cites them
    /// (a failed run keeps its evidence too).
    pub fn record_session(&mut self, record: &Value) {
        self.sessions.push(record.clone());
    }
    /// A passed guest acceptance suite (spec section 86) and its reference
    /// comparison (`not_applicable` when the suite compares nothing).
    pub fn record_suite(&mut self, suite: &str, reference_comparison: Value) {
        self.guest_suites
            .push(json!({"suite":suite,"reference_comparison":reference_comparison}));
    }
    /// Top-level report states derived from what actually ran, so they never
    /// contradict the guest result or the acceptance suites.
    fn states(&self, failure: Option<&str>) -> (&'static str, &'static str, Value) {
        let guest_tests = match &self.guest_result {
            Some(result) if result["outcome"] == "Passed" => "guest_tested",
            Some(_) => "attempted",
            None if self.guest_suites.is_empty() => "not_run",
            None if failure.is_none() => "guest_tested",
            None => "attempted",
        };
        let verification = if guest_tests == "guest_tested" && failure.is_none() {
            "guest_tested"
        } else {
            "host_tested"
        };
        let reference = if self.reference_comparison == "not_run" && !self.guest_suites.is_empty() {
            let suites: serde_json::Map<String, Value> = self
                .guest_suites
                .iter()
                .map(|s| {
                    (
                        s["suite"].as_str().unwrap_or("").to_owned(),
                        s["reference_comparison"].clone(),
                    )
                })
                .collect();
            let compared = suites
                .values()
                .any(|v| v != "not_applicable" && !v.is_null());
            json!({"state":if compared {"per_suite"} else {"not_applicable"},"suites":suites})
        } else {
            self.reference_comparison.clone()
        };
        (verification, guest_tests, reference)
    }

    pub fn finish(&self, failure: Option<&str>) -> Result<(), String> {
        if self.forked {
            return Err("a forked ledger is absorbed by its run, never finished".into());
        }
        let end_snapshot = snapshot(&self.root)?;
        let inputs_unchanged =
            self.start_snapshot["content_sha256"] == end_snapshot["content_sha256"];
        let input_error =
            "source inputs changed during the run; results cannot qualify the final source tree";
        let failure = failure.or(if inputs_unchanged {
            None
        } else {
            Some(input_error)
        });
        write_json(
            &self.root.join("reports/source-snapshot.json"),
            &end_snapshot,
        )?;
        // `out/` is cleaned: copy every cited serial log, staged image record
        // and screenshot into this run; `retained_evidence` cites the copies.
        let mut retained = std::collections::BTreeMap::new();
        for value in self.checks.iter().chain(&self.sessions) {
            retain_value(&self.root, &self.id, value, &mut retained)?;
        }
        if let Some(path) = self.guest_result.as_ref().and_then(|r| r["path"].as_str()) {
            let record = crate::image::read_json(&self.root.join(path))?;
            retain_value(&self.root, &self.id, &record, &mut retained)?;
        }
        let retained = retained
            .into_iter()
            .map(|(original, copy)| {
                Ok(json!({"original":original,"path":copy,"sha256":file_hash(&self.root.join(&copy))?}))
            })
            .collect::<Result<Vec<Value>, String>>()?;
        let (verification_state, guest_tests, reference_comparison) = self.states(failure);
        let mut report = json!({
            "schema_version": 1, "record_kind": "host_run", "id": self.id,
            "output_layout": self.output_layout.declaration(&self.id),
        "action": std::env::args().skip(1).collect::<Vec<_>>(),
            "verification_state": verification_state, "success": failure.is_none(), "failure": failure,
            "commands": self.commands, "source_snapshot": self.start_snapshot,
            "checks":self.checks,
            "certified_phase":self.certified_phase,
            "source_snapshot_after": end_snapshot, "inputs_unchanged": inputs_unchanged,
            "guest_tests":guest_tests,"guest_suites":self.guest_suites,
            "guest_result":self.guest_result,"reference_comparison": reference_comparison,
            "retained_evidence":retained
        });
        // Only runs that timed or scheduled anything carry these records;
        // every other report keeps its exact field set.
        if !self.timings.is_empty() {
            report["timings"] = json!(self.timings);
        }
        if let Some(scheduling) = &self.scheduling {
            report["scheduling"] = scheduling.clone();
        }
        write_json(&self.directory.join("report.json"), &report)?;
        println!("Evidence: {}", self.directory.join("report.json").display());
        if inputs_unchanged {
            Ok(())
        } else {
            Err(input_error.into())
        }
    }
}

fn required_timeout(program: &str, args: &[&str]) -> Duration {
    // The complete workspace suite includes hundreds of high-density UI rasters
    // and independent 128 MiB raw audits. Two progressing P12 attempts exceeded
    // the old 300s outer budget; no individual assertion/guest deadline changes.
    Duration::from_secs(
        if program == "cargo" && args == ["test", "--workspace", "--locked", "--all-targets"] {
            600
        } else {
            120
        },
    )
}

const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;
pub struct ControlledProcess<'a> {
    child: Option<Child>,
    started: Instant,
    stdout: &'a Path,
    stderr: &'a Path,
    watched: &'a [(&'a Path, u64)],
    exit: Option<ExitStatus>,
    handed_off: bool,
    timed_out: bool,
    output_limit_exceeded: bool,
    cleanup_error: Option<String>,
    output_root: &'a Path,
    budget_checked: Instant,
    output_budget_error: Option<String>,
}
impl ControlledProcess<'_> {
    pub fn id(&self) -> u32 {
        self.child.as_ref().expect("spawned child").id()
    }
    pub fn poll(&mut self) -> Result<Option<ExitStatus>, String> {
        if self.budget_checked.elapsed() >= Duration::from_secs(1) {
            self.output_budget_error = crate::cache::writer_headroom(self.output_root).err();
            self.budget_checked = Instant::now();
        }
        if let Some(error) = &self.output_budget_error {
            return Err(error.clone());
        }
        self.output_limit_exceeded |=
            log_limit(self.stdout, self.stderr) || watched_limit(self.watched);
        self.timed_out |= self.started.elapsed() > Duration::from_secs(90);
        if self.output_limit_exceeded {
            return Err("interactive process output limit".into());
        }
        if self.timed_out {
            return Err("interactive process timeout".into());
        }
        self.exit = self
            .child
            .as_mut()
            .ok_or("child not spawned")?
            .try_wait()
            .map_err(|e| e.to_string())?;
        Ok(self.exit)
    }
    pub fn wait(&mut self) -> Result<ExitStatus, String> {
        loop {
            if let Some(exit) = self.poll()? {
                return Ok(exit);
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
    pub fn handoff(&mut self) -> Result<(), String> {
        if self.poll()?.is_some() {
            return Err("child exited before owner handoff".into());
        }
        self.handed_off = true;
        Ok(())
    }
    fn terminate(&mut self) {
        if let Some(child) = &mut self.child {
            #[cfg(windows)]
            {
                let result = Command::new("taskkill.exe")
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                if !result.is_ok_and(|s| s.success()) {
                    self.cleanup_error = Some("interactive process tree cleanup failed".into());
                }
            }
            if let Err(e) = child.kill() {
                self.cleanup_error.get_or_insert(e.to_string());
            }
            match child.wait() {
                Ok(s) => self.exit = Some(s),
                Err(e) => {
                    self.cleanup_error.get_or_insert(e.to_string());
                }
            }
        }
    }
}
struct ProcessPolicy<'a> {
    directory: Option<&'a Path>,
    timeout: Duration,
    environment: &'a [(&'a str, &'a str)],
    required: bool,
    expected_exit: i32,
    watched: &'a [(&'a Path, u64)],
}
fn watched_limit(outputs: &[(&Path, u64)]) -> bool {
    outputs
        .iter()
        .any(|(path, limit)| fs::metadata(path).is_ok_and(|metadata| metadata.len() > *limit))
}
fn log_limit(stdout: &Path, stderr: &Path) -> bool {
    [stdout, stderr]
        .iter()
        .any(|path| fs::metadata(path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES))
}

pub fn resolve_tool(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        if path.extension().is_none() {
            let candidate = directory.join(format!("{name}.exe"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    })
}

/// Guest evidence a report may cite under `out/`, which is cleaned: serial
/// logs, staged image records and screenshots. Raw disks and build trees are
/// identified by hash only and never copied.
fn retainable(path: &str) -> bool {
    path.starts_with("out/")
        && !path.split('/').any(|part| part == ".." || part.is_empty())
        && (path.ends_with("/serial.log")
            || path.ends_with("/image.json")
            || path.ends_with(".png")
            || path.ends_with(".ppm"))
}
/// Where run `id` keeps its copy of `original` (`out/` replaced by
/// `reports/runs/<id>/retained/`).
pub fn retained_path(id: &str, original: &str) -> Option<String> {
    retainable(original).then(|| format!("reports/runs/{id}/retained/{}", &original[4..]))
}
/// Copies `original` (relative to `root`) into run `id`'s retained tree and
/// returns the copy's relative path; `None` for paths that are not retainable
/// or no longer exist. `replace` refreshes an existing copy (sealing).
pub fn retain(
    root: &Path,
    id: &str,
    original: &str,
    replace: bool,
) -> Result<Option<String>, String> {
    let Some(copy) = retained_path(id, original) else {
        return Ok(None);
    };
    let source = root.join(original);
    if !source.is_file() {
        return Ok(None);
    }
    let target = root.join(&copy);
    if replace || !target.is_file() {
        fs::create_dir_all(target.parent().ok_or("retained parent")?).map_err(|e| e.to_string())?;
        fs::copy(&source, &target).map_err(|e| format!("retain {original}: {e}"))?;
    }
    Ok(Some(copy))
}
/// Seals a guest session's serial: copies the live `out/` log into the run
/// that owns the session (`layout_owner`) as `serial_retained`, and records
/// the copy's hash as `serial_sha256`. `serial` keeps naming the live
/// session log, whose private directory the launch validators inspect.
/// Returns the sealed hash.
pub fn seal_session_serial(root: &Path, record: &mut Value) -> Result<String, String> {
    let original = record["serial"]
        .as_str()
        .ok_or("session serial")?
        .to_owned();
    let copy = match record["layout_owner"].as_str() {
        Some(owner) => retain(root, owner, &original, true)?,
        None => None,
    };
    let hash = file_hash(&root.join(copy.as_deref().unwrap_or(&original)))?;
    if let Some(copy) = copy {
        record["serial_retained"] = json!(copy);
    }
    record["serial_sha256"] = json!(hash);
    Ok(hash)
}
/// Copies every retainable `out/` path cited inside `value` into run `id`
/// (once), mapping each original to its copy. Cited strings are left as
/// they are: launch validators still inspect the live tree while it exists,
/// and the report's `retained_evidence` names the copies that outlive it.
fn retain_value(
    root: &Path,
    id: &str,
    value: &Value,
    retained: &mut std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    match value {
        Value::String(text) => {
            if !retained.contains_key(text.as_str())
                && let Some(copy) = retain(root, id, text, false)?
            {
                retained.insert(text.clone(), copy);
            }
        }
        Value::Array(items) => {
            for item in items {
                retain_value(root, id, item, retained)?;
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                retain_value(root, id, item, retained)?;
            }
        }
        _ => (),
    }
    Ok(())
}

/// Largest `out/latest-*` pointer, as large as the readers accept (`image::
/// read_json`, 512 MiB): the desktop record carries per-run source snapshots.
const LATEST_BOUND: usize = 512 * 1024 * 1024;
/// Whether `DOOR_XTASK_DEV_CACHE=1` asks for persistent development target
/// directories (`Run::dev_cache`).
pub fn dev_cache_enabled() -> bool {
    std::env::var("DOOR_XTASK_DEV_CACHE").is_ok_and(|v| v == "1")
}
pub fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path.parent().ok_or("missing parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    // v2 desktop's only out writes are bounded replaceable latest pointers.
    // Bound staging as well as the final file, before creating either one.
    if parent.file_name().is_some_and(|name| name == "out")
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("latest-"))
        && bytes.len() > LATEST_BOUND
    {
        return Err("latest output metadata exceeds 512 MiB bound".into());
    }
    // Same-directory staging followed by replacement. Generated reports only;
    // this helper never replaces the specification or source schemas.
    // Concurrent workers of one process stage under distinct names and replace
    // one at a time (several child runs refresh the same source snapshot).
    static STAGING: AtomicU64 = AtomicU64::new(0);
    static REPLACE: Mutex<()> = Mutex::new(());
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        STAGING.fetch_add(1, Ordering::Relaxed)
    ));
    File::create(&temporary)
        .and_then(|mut f| f.write_all(&bytes))
        .map_err(|e| e.to_string())?;
    let _replacing = REPLACE.lock().unwrap_or_else(PoisonError::into_inner);
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    fs::rename(&temporary, path).map_err(|e| e.to_string())
}

pub fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn relative(root: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .map_err(|e| e.to_string())
}

pub(crate) fn snapshot(root: &Path) -> Result<Value, String> {
    let mut paths = Vec::new();
    collect(root, root, &mut paths)?;
    let digests = hash_all(&paths)?;
    let mut files: Vec<Value> = paths
        .into_iter()
        .zip(digests)
        .map(|((relative, _), sha256)| json!({"path": relative, "sha256": sha256}))
        .collect();
    files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let manifest_bytes = serde_json::to_vec(&files).map_err(|e| e.to_string())?;
    Ok(
        json!({"algorithm": "sha256", "manifest_encoding": "compact_json_sorted_paths",
        "content_sha256": format!("{:x}", Sha256::digest(manifest_bytes)), "files": files,
        "scope": "workspace source; excludes .git, target, out and generated local reports",
        "off_device_backup": false, "git_commit": null}),
    )
}

/// SHA-256 of every file, in input order (a snapshot covers every source byte
/// twice per run). One snapshot hashes at a time in this process, on a
/// quarter of the logical processors, so concurrently running guests keep
/// the rest. The first failure in input order is reported, as a sequential
/// walk would.
fn hash_all(paths: &[(String, PathBuf)]) -> Result<Vec<String>, String> {
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _hashing = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let workers = (thread::available_parallelism().map_or(1, usize::from) / 4)
        .clamp(1, 8)
        .min(paths.len().max(1));
    let next = AtomicUsize::new(0);
    let mut results: Vec<(usize, Result<String, String>)> = thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some((_, path)) = paths.get(index) else {
                            break done;
                        };
                        done.push((index, file_hash(path)));
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    });
    if results.len() != paths.len() {
        return Err("source snapshot hashing worker failed".into());
    }
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, digest)| digest).collect()
}

fn collect(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let relative = relative(root, &path)?;
        if matches!(
            relative.as_str(),
            ".git"
                | "target"
                | "out"
                | "reports/runs"
                | "reports/environment.json"
                | "reports/source-snapshot.json"
        ) {
            continue;
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err(format!("source snapshot refuses symlink: {relative}"));
        }
        if kind.is_dir() {
            collect(root, &path, files)?;
        } else if kind.is_file() {
            files.push((relative, path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_complete_workspace_tests_receive_the_storage_audit_budget() {
        assert_eq!(
            super::required_timeout(
                "cargo",
                &["test", "--workspace", "--locked", "--all-targets"]
            )
            .as_secs(),
            600
        );
        for (program, args) in [
            (
                "cargo",
                vec!["check", "--workspace", "--locked", "--all-targets"],
            ),
            ("cargo", vec!["test", "--workspace", "--locked"]),
            (
                "cargo",
                vec!["test", "--workspace", "--locked", "--all-targets", "extra"],
            ),
            (
                "other",
                vec!["test", "--workspace", "--locked", "--all-targets"],
            ),
        ] {
            assert_eq!(super::required_timeout(program, &args).as_secs(), 120);
        }
    }
    use super::*;

    fn isolated_run() -> Run {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("door-xtask-test-{stamp}-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Run::new(root).unwrap()
    }

    #[test]
    fn guest_evidence_outlives_out_and_report_states_follow_the_suites() {
        let mut run = isolated_run();
        let session = run.root.join("out/qemu/s1");
        fs::create_dir_all(&session).unwrap();
        fs::create_dir_all(run.root.join("out/images/x")).unwrap();
        fs::write(session.join("serial.log"), "DOOR_P08 CLEANUP PASS\n").unwrap();
        fs::write(session.join("shell.png"), "png").unwrap();
        fs::write(session.join("data-after.raw"), "raw").unwrap();
        fs::write(run.root.join("out/images/x/image.json"), "{}").unwrap();
        let mut record = json!({"serial":"out/qemu/s1/serial.log","layout_owner":run.id,
            "screenshot":"out/qemu/s1/shell.png","image":"out/images/x/image.json"});
        let sealed = seal_session_serial(&run.root, &mut record).unwrap();
        let copy = format!("reports/runs/{}/retained/qemu/s1/serial.log", run.id);
        assert_eq!(record["serial_retained"], copy.as_str());
        assert_eq!(record["serial"], "out/qemu/s1/serial.log");
        assert_eq!(record["serial_sha256"], sealed.as_str());
        // Later appends to the live log do not change the sealed copy.
        fs::write(session.join("serial.log"), "DOOR_P08 CLEANUP PASS\nlate\n").unwrap();
        assert_eq!(file_hash(&run.root.join(&copy)).unwrap(), sealed);
        run.check(
            "suite",
            json!({"session":record,"disk":"out/qemu/s1/data-after.raw",
                "escape":"out/../escape.png","missing":"out/qemu/s9/serial.log"}),
        );
        run.record_suite("win32-gui-f1", json!({"state":"passed"}));
        run.record_suite("win32-gui-f3", json!("not_applicable"));
        run.finish(None).unwrap();
        fs::remove_dir_all(run.root.join("out")).unwrap();
        let report: Value =
            serde_json::from_slice(&fs::read(run.directory.join("report.json")).unwrap()).unwrap();
        let retained = report["retained_evidence"].as_array().unwrap();
        let originals: Vec<&str> = retained
            .iter()
            .map(|r| r["original"].as_str().unwrap())
            .collect();
        assert_eq!(
            originals,
            [
                "out/images/x/image.json",
                "out/qemu/s1/serial.log",
                "out/qemu/s1/shell.png"
            ]
        );
        for entry in retained {
            let path = run.root.join(entry["path"].as_str().unwrap());
            assert_eq!(file_hash(&path).unwrap(), entry["sha256"].as_str().unwrap());
        }
        // The serial copy is the sealed one, not refreshed at finish.
        assert_eq!(retained[1]["sha256"], sealed.as_str());
        assert_eq!(report["verification_state"], "guest_tested");
        assert_eq!(report["guest_tests"], "guest_tested");
        assert_eq!(report["reference_comparison"]["state"], "per_suite");
        assert_eq!(
            report["reference_comparison"]["suites"]["win32-gui-f1"]["state"],
            "passed"
        );
        fs::remove_dir_all(&run.root).unwrap();

        let mut failed = isolated_run();
        failed.record_suite("win32-gui-f1", json!("not_applicable"));
        // A session of a failed run, never sealed or cited: still retained.
        fs::create_dir_all(failed.root.join("out/qemu/s2")).unwrap();
        fs::write(failed.root.join("out/qemu/s2/serial.log"), "KERNEL_PANIC\n").unwrap();
        failed.record_session(&json!({"serial":"out/qemu/s2/serial.log"}));
        failed.finish(Some("later suite failed")).unwrap();
        let report: Value =
            serde_json::from_slice(&fs::read(failed.directory.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(report["verification_state"], "host_tested");
        assert_eq!(report["guest_tests"], "attempted");
        assert_eq!(report["reference_comparison"]["state"], "not_applicable");
        assert_eq!(
            report["retained_evidence"][0]["path"],
            format!("reports/runs/{}/retained/qemu/s2/serial.log", failed.id)
        );
        fs::remove_dir_all(&failed.root).unwrap();
    }

    #[test]
    fn oversized_latest_metadata_cannot_replace_the_pointer_or_leave_staging() {
        let workspace = crate::test_support::Workspace::new();
        let path = workspace.root.join("out/latest-desktop-image.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"original").unwrap();
        assert!(write_json(&path, &json!("x".repeat(LATEST_BOUND))).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn forked_ledgers_share_numbering_and_merge_in_caller_order() {
        let mut run = isolated_run();
        let before = run.id.clone();
        run.optional("door-nonexistent-tool-1", &[]).unwrap();
        let mut first = run.fork_ledger();
        let mut second = run.fork_ledger();
        assert_eq!(
            (first.id.as_str(), &first.directory),
            (before.as_str(), &run.directory)
        );
        second.optional("door-nonexistent-tool-3", &[]).unwrap();
        first.optional("door-nonexistent-tool-2", &[]).unwrap();
        first.check("from-first", json!({}));
        second.check("from-second", json!({}));
        first.record_timing("suite", "first", Duration::from_millis(5), "passed");
        assert!(
            first.finish(None).is_err(),
            "a forked ledger never finishes"
        );
        run.absorb(first);
        run.absorb(second);
        let ids: Vec<_> = run.commands().iter().map(|c| c["id"].clone()).collect();
        assert_eq!(ids, ["command-01", "command-03", "command-02"]);
        for command in run.commands() {
            let log = run.root.join(command["stderr"]["path"].as_str().unwrap());
            assert_eq!(file_hash(&log).unwrap(), command["stderr"]["sha256"]);
        }
        let checks: Vec<_> = run.checks().iter().map(|c| c["id"].clone()).collect();
        assert_eq!(checks, ["from-first", "from-second"]);
        assert!(run.timing_summary(5).contains("first"));
        run.finish(None).unwrap();
        let report: Value =
            serde_json::from_slice(&fs::read(run.directory.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["timings"][0]["name"], "first");
        assert!(report.get("scheduling").is_none());
        fs::remove_dir_all(&run.root).unwrap();
        // A run that timed nothing keeps its exact report fields.
        let plain = isolated_run();
        plain.finish(None).unwrap();
        let report: Value =
            serde_json::from_slice(&fs::read(plain.directory.join("report.json")).unwrap())
                .unwrap();
        assert!(report.get("timings").is_none());
        fs::remove_dir_all(&plain.root).unwrap();
    }

    #[test]
    fn concurrent_runs_and_writers_never_share_identity_or_staging() {
        let stamps: Vec<u128> = (0..1000).map(|_| unique_stamp().unwrap()).collect();
        assert!(stamps.windows(2).all(|w| w[0] < w[1]));
        let workspace = crate::test_support::Workspace::new();
        let path = workspace.root.join("shared.json");
        thread::scope(|scope| {
            for writer in 0..8 {
                let path = &path;
                scope.spawn(move || {
                    for round in 0..25 {
                        write_json(path, &json!({"writer":writer,"round":round})).unwrap();
                    }
                });
            }
        });
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["round"], 24);
        assert_eq!(fs::read_dir(&workspace.root).unwrap().count(), 1);
    }

    #[test]
    fn missing_tool_is_recorded_and_never_passes() {
        let mut run = isolated_run();
        let observation = run.optional("door-nonexistent-tool-410921", &[]).unwrap();
        assert!(!observation.success);
        assert_eq!(run.commands()[0]["outcome"], "tool_error");
        assert!(run.commands()[0]["exit_code"].is_null());
        fs::remove_dir_all(&run.root).unwrap();
    }

    #[test]
    fn nonzero_exit_keeps_stderr_and_hashes() {
        let mut run = isolated_run();
        let observation = run.optional("rustc", &["--door-invalid-option"]).unwrap();
        assert!(!observation.success);
        assert!(!observation.stderr.is_empty());
        assert_eq!(run.commands()[0]["outcome"], "failed");
        let log = run
            .root
            .join(run.commands()[0]["stderr"]["path"].as_str().unwrap());
        assert_eq!(
            file_hash(&log).unwrap(),
            run.commands()[0]["stderr"]["sha256"].as_str().unwrap()
        );
        fs::remove_dir_all(&run.root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn controlled_environment_is_child_only_and_bound_to_its_stream() {
        let mut run = isolated_run();
        let before = std::env::var_os("SDL_WINDOWS_DPI_AWARENESS");
        let scaling_before = std::env::var_os("SDL_WINDOWS_DPI_SCALING");
        let args = ["-NoProfile", "-NonInteractive", "-Command",
            "[Console]::Out.WriteLine($env:SDL_WINDOWS_DPI_AWARENESS); [Console]::Out.WriteLine($env:SDL_WINDOWS_DPI_SCALING)"]
            .map(String::from);
        let overrides = [
            ("SDL_WINDOWS_DPI_AWARENESS", "permonitorv2"),
            ("SDL_WINDOWS_DPI_SCALING", "0"),
        ];
        run.controlled_expected_env("powershell.exe", &args, &[], 0, &overrides, |child| {
            child.wait().map(|_| ())
        })
        .unwrap();
        assert_eq!(std::env::var_os("SDL_WINDOWS_DPI_AWARENESS"), before);
        assert_eq!(std::env::var_os("SDL_WINDOWS_DPI_SCALING"), scaling_before);
        let record = &run.commands()[0];
        assert_eq!(record["environment_overrides"], json!(overrides));
        let path = run.root.join(record["stdout"]["path"].as_str().unwrap());
        assert_eq!(
            fs::read_to_string(&path).unwrap().replace('\r', ""),
            "permonitorv2\n0\n"
        );
        assert_eq!(file_hash(&path).unwrap(), record["stdout"]["sha256"]);
        fs::remove_dir_all(&run.root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn interactive_process_records_completion_and_reaps_limit_failure() {
        let mut run = isolated_run();
        let args = ["-NoProfile", "-NonInteractive", "-Command", "exit 7"].map(String::from);
        assert!(
            run.controlled("powershell.exe", &args, &[], |child| {
                child.wait().map(|_| ())
            })
            .is_err()
        );
        assert_eq!(run.commands()[0]["exit_code"], 7);
        assert_eq!(run.commands()[0]["lifecycle"], "completed");
        assert_eq!(run.commands()[0]["outcome"], "failed");
        let args = ["-NoProfile", "-NonInteractive", "-Command", "exit 33"].map(String::from);
        run.controlled_expected("powershell.exe", &args, &[], 33, |child| {
            child.wait().map(|_| ())
        })
        .unwrap();
        assert_eq!(run.commands()[1]["expected_exit_code"], 33);
        assert_eq!(run.commands()[1]["exit_code"], 33);
        assert_eq!(run.commands()[1]["outcome"], "passed");
        let watched = run.directory.join("interactive.log");
        fs::write(&watched, [0; 2048]).unwrap();
        let args = [
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 30",
        ]
        .map(String::from);
        assert!(
            run.controlled("powershell.exe", &args, &[(&watched, 1024)], |child| {
                child.wait().map(|_| ())
            })
            .is_err()
        );
        let record = &run.commands()[2];
        assert_eq!(record["output_limit_exceeded"], true);
        assert!(!record["exit_code"].is_null());
        assert_eq!(fs::metadata(watched).unwrap().len(), 1024);
        for stream in ["stdout", "stderr"] {
            assert_eq!(
                file_hash(&run.root.join(record[stream]["path"].as_str().unwrap())).unwrap(),
                record[stream]["sha256"]
            );
        }
        fs::remove_dir_all(&run.root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn timeout_is_recorded_and_child_is_reaped() {
        let mut run = isolated_run();
        let observation = run
            .command(
                "powershell.exe",
                &[
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 30",
                ],
                false,
                Duration::from_millis(100),
            )
            .unwrap();
        assert!(!observation.success);
        assert_eq!(run.commands()[0]["outcome"], "timeout");
        assert_eq!(run.commands()[0]["timed_out"], true);
        assert!(!run.commands()[0]["exit_code"].is_null());
        fs::remove_dir_all(&run.root).unwrap();
    }

    #[test]
    fn snapshot_changes_with_source_and_excludes_outputs() {
        let run = isolated_run();
        fs::write(run.root.join("source.rs"), b"first").unwrap();
        let before = snapshot(&run.root).unwrap();
        fs::write(run.directory.join("log.txt"), b"generated").unwrap();
        assert_eq!(before, snapshot(&run.root).unwrap());
        fs::write(run.root.join("source.rs"), b"second").unwrap();
        assert_ne!(
            before["content_sha256"],
            snapshot(&run.root).unwrap()["content_sha256"]
        );
        fs::remove_dir_all(&run.root).unwrap();
    }

    #[test]
    fn changing_source_after_checks_produces_failed_evidence() {
        let run = isolated_run();
        fs::write(
            run.root.join("source.rs"),
            b"compile_error!(\"late edit\");",
        )
        .unwrap();
        assert!(run.finish(None).is_err());
        let report: Value =
            serde_json::from_slice(&fs::read(run.directory.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["success"], false);
        assert_eq!(report["inputs_unchanged"], false);
        assert_ne!(
            report["source_snapshot"]["content_sha256"],
            report["source_snapshot_after"]["content_sha256"]
        );
        fs::remove_dir_all(&run.root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn oversized_output_is_failed_and_captured_stream_is_capped() {
        let mut run = isolated_run();
        let observation = run
            .optional(
                "powershell.exe",
                &[
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "[Console]::Write('x' * 9000000)",
                ],
            )
            .unwrap();
        assert!(!observation.success);
        assert!(observation.output_limit_exceeded);
        let path = run
            .root
            .join(run.commands()[0]["stdout"]["path"].as_str().unwrap());
        assert_eq!(fs::metadata(path).unwrap().len(), MAX_LOG_BYTES);
        fs::remove_dir_all(&run.root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn expected_debug_exit_and_aggregate_guest_record_stay_consistent() {
        let mut run = isolated_run();
        let observed = run
            .watched(
                "powershell.exe",
                &["-NoProfile", "-NonInteractive", "-Command", "exit 33"],
                Duration::from_secs(5),
                33,
                &[],
            )
            .unwrap();
        assert!(observed.success);
        assert_eq!(run.commands()[0]["expected_exit_code"], 33);
        run.record_guest(&json!({"record_kind":"guest_run","outcome":"Passed","verification_state":"guest_tested","scope":"host-test-simulation"})).unwrap();
        run.finish(None).unwrap();
        let report: Value =
            serde_json::from_slice(&fs::read(run.directory.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["guest_tests"], "guest_tested");
        assert_eq!(report["guest_result"]["outcome"], "Passed");
        let path = run
            .root
            .join(report["guest_result"]["path"].as_str().unwrap());
        assert_eq!(
            file_hash(&path).unwrap(),
            report["guest_result"]["sha256"].as_str().unwrap()
        );
        fs::remove_dir_all(&run.root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn failed_guest_attempt_remains_failed_and_bound_in_aggregate() {
        let mut run = isolated_run();
        let observed = run
            .watched(
                "powershell.exe",
                &["-NoProfile", "-NonInteractive", "-Command", "exit 33"],
                Duration::from_secs(5),
                33,
                &[],
            )
            .unwrap();
        assert!(observed.success);
        run.record_guest(&json!({"record_kind":"guest_run","outcome":"KernelPanic","verification_state":"attempted","scope":"host-test-simulation"})).unwrap();
        run.finish(Some("guest run KernelPanic")).unwrap();
        let report: Value =
            serde_json::from_slice(&fs::read(run.directory.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["success"], false);
        assert_eq!(report["guest_tests"], "attempted");
        assert_eq!(report["guest_result"]["verification_state"], "attempted");
        assert_eq!(report["guest_result"]["outcome"], "KernelPanic");
        let path = run
            .root
            .join(report["guest_result"]["path"].as_str().unwrap());
        assert_eq!(
            file_hash(&path).unwrap(),
            report["guest_result"]["sha256"].as_str().unwrap()
        );
        fs::remove_dir_all(&run.root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn watched_serial_is_bounded_during_child_execution() {
        let mut run = isolated_run();
        let serial = run.directory.join("serial.log");
        let path = serial.to_str().unwrap();
        let observed=run.execute("powershell.exe",&["-NoProfile","-NonInteractive","-Command","[IO.File]::WriteAllText($env:DOOR_TEST_SERIAL, 'x' * 4096); Start-Sleep -Seconds 30"],
            ProcessPolicy{directory:None,timeout:Duration::from_secs(5),environment:&[("DOOR_TEST_SERIAL",path)],required:false,expected_exit:0,watched:&[(&serial,1024)]}).unwrap();
        assert!(!observed.success);
        assert!(observed.output_limit_exceeded);
        assert!(!observed.timed_out);
        assert_eq!(fs::metadata(&serial).unwrap().len(), 1024);
        fs::remove_dir_all(&run.root).unwrap();
    }
}
