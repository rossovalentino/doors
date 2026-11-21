//! Bounded worker pool for independent guest work (`DOOR_GUEST_JOBS`).
//!
//! The cumulative gate runs its desktop gate steps, desktop scenarios and
//! acceptance suites on at most `jobs` workers. Every worker owns private
//! guest state: a child run (desktop scenarios) or a forked ledger of the
//! parent run (`Run::fork_ledger`: gate steps and suites), its own raw disks,
//! its own QEMU session directory (private ESP copy, firmware variables and
//! serial log, named by a process-unique instance stamp) and its own QMP port
//! (`session::reserve_qmp_port`). Session retirement never touches a sibling:
//! it skips every session launched by this process and runs only while no
//! QEMU is active (`emulator::retire_finished_esps`).
//!
//! Timing-sensitive work runs in an exclusive slot with no other guest
//! running. `DOOR_GUEST_JOBS=1` runs every task in declared order, stopping
//! at the first failure, exactly as the sequential gate always did.
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub const JOBS_ENV: &str = "DOOR_GUEST_JOBS";
/// `DOOR_GUEST_KEEP_GOING=1`: a failed task no longer stops the pool, so one
/// run reports every failing scenario (owner, 14 November 2026: each gate
/// attempt otherwise surfaced one problem per hour). The run still fails if
/// any task failed; nothing is skipped silently.
pub const KEEP_GOING_ENV: &str = "DOOR_GUEST_KEEP_GOING";
fn keep_going() -> bool {
    // Unit tests check the default stop-at-first-failure behaviour; a gate
    // started with the variable set runs them too.
    !cfg!(test) && std::env::var(KEEP_GOING_ENV).is_ok_and(|v| v == "1")
}
/// Every desktop hardware profile pins a 2048 MiB guest.
const GUEST_MIB: u64 = 2048;
/// QEMU's own allocations beside guest RAM (TCG translation buffer, VGA or
/// RAMFB framebuffers, vvfat) plus the worker's host-side frame buffers.
const WORKER_OVERHEAD_MIB: u64 = 1536;
/// Left for Windows, the orchestrator itself and the owner's applications.
const HOST_RESERVE_MIB: u64 = 4096;
/// A TCG guest keeps its vCPU thread and QEMU main loop busy; the worker's
/// capture comparisons take a third core in bursts.
const CORES_PER_WORKER: usize = 3;
const RESERVED_CORES: usize = 2;
/// Never schedule work into the last 25 GB of the output volume.
const DISK_RESERVE_BYTES: u64 = 25_000_000_000;
/// In-flight bytes of one worker: raw disks and their snapshots, the private
/// ESP copy and full-resolution PPM captures.
const DISK_PER_WORKER_BYTES: u64 = 3_000_000_000;
const MAX_DEFAULT_JOBS: usize = 8;

/// The guest worker count and how it was chosen (recorded in the report).
pub struct Concurrency {
    pub jobs: usize,
    pub record: Value,
}

/// `DOOR_GUEST_JOBS=N` when set, else the default from free memory, logical
/// processors and free disk on the output volume.
pub fn concurrency(root: &Path) -> Result<Concurrency, String> {
    if let Some(value) = std::env::var_os(JOBS_ENV) {
        let text = value.to_string_lossy().into_owned();
        let jobs = text
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=64).contains(n))
            .ok_or_else(|| {
                format!("{JOBS_ENV} must be a whole number from 1 to 64, got {text:?}")
            })?;
        return Ok(Concurrency {
            jobs,
            record: json!({"jobs":jobs,"source":JOBS_ENV}),
        });
    }
    let cores = thread::available_parallelism().map_or(1, usize::from);
    Ok(match host_resources(root) {
        Ok((free_mib, free_disk)) => {
            let (jobs, limits) = default_jobs(cores, free_mib, free_disk);
            Concurrency {
                jobs,
                record: json!({"jobs":jobs,"source":"default","logical_processors":cores,
                    "free_memory_mib":free_mib,"free_output_disk_bytes":free_disk,"limits":limits,
                    "formula":format!("min(({cores} cores - {RESERVED_CORES}) / {CORES_PER_WORKER}, (free MiB - {HOST_RESERVE_MIB}) / ({GUEST_MIB} guest + {WORKER_OVERHEAD_MIB} overhead), (free disk - 25 GB) / 3 GB), clamped to 1..={MAX_DEFAULT_JOBS}")}),
            }
        }
        Err(error) => Concurrency {
            jobs: 1,
            record: json!({"jobs":1,"source":"fallback","reason":error}),
        },
    })
}

/// The default worker count and each resource's own limit.
pub fn default_jobs(cores: usize, free_mib: u64, free_disk: u64) -> (usize, Value) {
    let by_cpu = cores.saturating_sub(RESERVED_CORES) / CORES_PER_WORKER;
    let by_memory = usize::try_from(
        free_mib.saturating_sub(HOST_RESERVE_MIB) / (GUEST_MIB + WORKER_OVERHEAD_MIB),
    )
    .unwrap_or(usize::MAX);
    let by_disk =
        usize::try_from(free_disk.saturating_sub(DISK_RESERVE_BYTES) / DISK_PER_WORKER_BYTES)
            .unwrap_or(usize::MAX);
    let jobs = by_cpu
        .min(by_memory)
        .min(by_disk)
        .clamp(1, MAX_DEFAULT_JOBS);
    (
        jobs,
        json!({"cpu":by_cpu,"memory":by_memory,"disk":by_disk,"maximum":MAX_DEFAULT_JOBS}),
    )
}

/// Free physical memory (MiB) and free bytes on the volume holding `root`.
fn host_resources(root: &Path) -> Result<(u64, u64), String> {
    #[cfg(windows)]
    {
        use std::{
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        let text = root.to_string_lossy();
        let text = text.trim_start_matches(r"\\?\");
        let drive = text
            .get(..2)
            .filter(|d| d.as_bytes()[0].is_ascii_alphabetic() && d.as_bytes()[1] == b':')
            .ok_or("output volume is not a drive letter")?;
        let script = format!(
            "$o = Get-CimInstance Win32_OperatingSystem; $d = Get-CimInstance Win32_LogicalDisk -Filter \"DeviceID='{drive}'\"; '{{0}} {{1}}' -f $o.FreePhysicalMemory, $d.FreeSpace"
        );
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x0800_0000)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("host resource probe: {e}"))?;
        if !output.status.success() {
            return Err("host resource probe failed".into());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut words = stdout.split_whitespace();
        let mut number = || -> Result<u64, String> {
            words
                .next()
                .and_then(|w| w.parse::<u64>().ok())
                .ok_or_else(|| format!("host resource probe output {stdout:?}"))
        };
        let free_kib = number()?;
        let free_disk = number()?;
        Ok((free_kib / 1024, free_disk))
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Err("host resource probe requires Windows".into())
    }
}

/// `DOOR_GUEST_JOBS=1`: every stage runs sequentially, as it always did.
pub fn sequential_requested() -> bool {
    std::env::var(JOBS_ENV).is_ok_and(|value| value.trim() == "1")
}

/// A build and its controlled rebuild into separate target directories:
/// two independent passes, run concurrently on forked ledgers of `run` and
/// absorbed in pass order (the order the sequential gate recorded them in).
/// With `DOOR_GUEST_JOBS=1` they run one after the other on `run`, and a
/// failed first pass stops before the second starts.
pub fn pair<A: Send, B: Send>(
    run: &mut crate::evidence::Run,
    first: impl FnOnce(&mut crate::evidence::Run) -> Result<A, String> + Send,
    second: impl FnOnce(&mut crate::evidence::Run) -> Result<B, String> + Send,
) -> Result<(A, B), String> {
    if sequential_requested() {
        let a = first(run)?;
        let b = second(run)?;
        return Ok((a, b));
    }
    let (mut left, mut right) = (run.fork_ledger(), run.fork_ledger());
    let (a, b) = thread::scope(|scope| {
        let a = scope.spawn(|| labelled("pass 1", || first(&mut left)));
        let b = scope.spawn(|| labelled("pass 2", || second(&mut right)));
        (
            a.join().unwrap_or_else(|panic| Err(panic_text(panic))),
            b.join().unwrap_or_else(|panic| Err(panic_text(panic))),
        )
    });
    run.absorb(left);
    run.absorb(right);
    Ok((a?, b?))
}

fn labelled<T>(name: &str, work: impl FnOnce() -> T) -> T {
    LABEL.with(|label| *label.borrow_mut() = Some(name.to_owned()));
    work()
}

thread_local! {
    static LABEL: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// `"[name] "` inside a pool worker, so interleaved console lines name their
/// task; empty on the orchestrator thread (sequential output is unchanged).
pub fn console_prefix() -> String {
    LABEL.with(|label| {
        label
            .borrow()
            .as_ref()
            .map_or_else(String::new, |name| format!("[{name}] "))
    })
}

/// One unit of independent guest work.
pub struct Task<'scope, T> {
    pub name: String,
    /// Runs with no other task (and no other guest) in flight.
    pub exclusive: bool,
    work: Box<dyn FnOnce() -> T + Send + 'scope>,
}

impl<'scope, T> Task<'scope, T> {
    pub fn new(
        name: impl Into<String>,
        exclusive: bool,
        work: impl FnOnce() -> T + Send + 'scope,
    ) -> Self {
        Self {
            name: name.into(),
            exclusive,
            work: Box::new(work),
        }
    }
}

/// What became of one task, in declared order.
pub struct Outcome<T> {
    pub name: String,
    pub exclusive: bool,
    /// `None`: never started, because an earlier failure stopped scheduling.
    /// `Some(Err)`: the task panicked.
    pub result: Option<Result<T, String>>,
    pub elapsed: Duration,
}

/// Runs `tasks` on at most `jobs` workers and returns their outcomes in
/// declared order. With one job the tasks run in declared order on this
/// thread. With more, exclusive tasks run first, each alone, then the rest
/// share the workers. After the first failure (`failed`, or a panic) nothing
/// new starts; running tasks finish and are reported.
pub fn run<'scope, T: Send + 'scope>(
    jobs: usize,
    tasks: Vec<Task<'scope, T>>,
    failed: impl Fn(&T) -> bool,
) -> Vec<Outcome<T>> {
    let mut outcomes: Vec<Outcome<T>> = tasks
        .iter()
        .map(|task| Outcome {
            name: task.name.clone(),
            exclusive: task.exclusive,
            result: None,
            elapsed: Duration::ZERO,
        })
        .collect();
    let total = tasks.len();
    let keep_going = keep_going();
    let mut work: Vec<Option<Box<dyn FnOnce() -> T + Send + 'scope>>> =
        tasks.into_iter().map(|task| Some(task.work)).collect();
    if jobs <= 1 {
        for (index, outcome) in outcomes.iter_mut().enumerate() {
            let task = work[index].take().expect("task runs once");
            let started = Instant::now();
            let result = catch_unwind(AssertUnwindSafe(task)).map_err(panic_text);
            outcome.elapsed = started.elapsed();
            let failure = match &result {
                Ok(value) => failed(value),
                Err(_) => true,
            };
            outcome.result = Some(result);
            if failure && !keep_going {
                break;
            }
        }
        return outcomes;
    }
    let mut queue: VecDeque<usize> = (0..total)
        .filter(|i| outcomes[*i].exclusive)
        .chain((0..total).filter(|i| !outcomes[*i].exclusive))
        .collect();
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel::<(usize, Duration, Result<T, String>)>();
        let mut running = 0usize;
        let mut exclusive_running = false;
        let mut stopped = false;
        let mut started_count = 0usize;
        loop {
            while !stopped && let Some(&next) = queue.front() {
                let exclusive = outcomes[next].exclusive;
                if (exclusive && running > 0) || exclusive_running || running >= jobs {
                    break;
                }
                queue.pop_front();
                let task = work[next].take().expect("task runs once");
                let name = outcomes[next].name.clone();
                started_count += 1;
                println!(
                    "[guest pool] start {name}{} ({started_count}/{total}, {} running)",
                    if exclusive { " (exclusive)" } else { "" },
                    running + 1
                );
                let sender = sender.clone();
                scope.spawn(move || {
                    LABEL.with(|label| *label.borrow_mut() = Some(name));
                    let started = Instant::now();
                    let result = catch_unwind(AssertUnwindSafe(task)).map_err(panic_text);
                    // The receiver outlives every worker of this scope.
                    let _ = sender.send((next, started.elapsed(), result));
                });
                running += 1;
                exclusive_running = exclusive;
            }
            if running == 0 {
                break;
            }
            let Ok((index, elapsed, result)) = receiver.recv() else {
                break;
            };
            running -= 1;
            if outcomes[index].exclusive {
                exclusive_running = false;
            }
            let failure = match &result {
                Ok(value) => failed(value),
                Err(_) => true,
            };
            println!(
                "[guest pool] {} {} in {:.1} s",
                outcomes[index].name,
                if failure { "FAILED" } else { "passed" },
                elapsed.as_secs_f64()
            );
            if failure && keep_going {
                println!("[guest pool] failure recorded; continuing ({KEEP_GOING_ENV}=1)");
            } else if failure && !stopped {
                stopped = true;
                println!(
                    "[guest pool] first failure: no new work starts; waiting for {running} running task(s)"
                );
            }
            outcomes[index].elapsed = elapsed;
            outcomes[index].result = Some(result);
        }
    });
    outcomes
}

fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
    let text = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-text panic".into());
    format!("task panicked: {text}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn default_concurrency_follows_the_scarcest_resource() {
        // The owner workstation: 24 logical processors, about 20 GiB free
        // memory and 78 GB free disk.
        let (jobs, limits) = default_jobs(24, 20 * 1024, 78_000_000_000);
        assert_eq!(jobs, 4);
        assert_eq!(limits["memory"], 4);
        assert_eq!(limits["cpu"], 7);
        assert_eq!(limits["disk"], 17);
        assert_eq!(default_jobs(4, 64 * 1024, 500_000_000_000).0, 1);
        assert_eq!(default_jobs(64, 2048, 500_000_000_000).0, 1);
        assert_eq!(default_jobs(64, 256 * 1024, 26_000_000_000).0, 1);
        assert_eq!(default_jobs(128, 1024 * 1024, 5_000_000_000_000).0, 8);
    }

    #[test]
    fn one_job_runs_in_declared_order_and_stops_at_the_first_failure() {
        let order = Mutex::new(Vec::new());
        let tasks: Vec<_> = (0..5)
            .map(|i| {
                let order = &order;
                Task::new(format!("t{i}"), i == 3, move || {
                    order.lock().unwrap().push(i);
                    i != 2
                })
            })
            .collect();
        let outcomes = run(1, tasks, |ok| !ok);
        assert_eq!(*order.lock().unwrap(), [0, 1, 2]);
        assert!(outcomes[..3].iter().all(|o| o.result.is_some()));
        assert!(outcomes[3..].iter().all(|o| o.result.is_none()));
        assert_eq!(
            outcomes[2].result.as_ref().unwrap().as_ref().ok(),
            Some(&false)
        );
    }

    #[test]
    fn exclusive_tasks_run_alone_and_the_rest_share_the_bound() {
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let violations = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..12)
            .map(|i| {
                let (running, peak, violations) =
                    (running.clone(), peak.clone(), violations.clone());
                let exclusive = i % 5 == 0;
                Task::new(format!("t{i}"), exclusive, move || {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    if exclusive && now != 1 {
                        violations.fetch_add(1, Ordering::SeqCst);
                    }
                    thread::sleep(Duration::from_millis(30));
                    if exclusive && running.load(Ordering::SeqCst) != 1 {
                        violations.fetch_add(1, Ordering::SeqCst);
                    }
                    running.fetch_sub(1, Ordering::SeqCst);
                    i
                })
            })
            .collect();
        let outcomes = run(3, tasks, |_| false);
        assert_eq!(violations.load(Ordering::SeqCst), 0);
        assert_eq!(peak.load(Ordering::SeqCst), 3);
        for (i, outcome) in outcomes.iter().enumerate() {
            assert_eq!(outcome.name, format!("t{i}"));
            assert_eq!(outcome.result.as_ref().unwrap().as_ref().ok(), Some(&i));
        }
    }

    #[test]
    fn a_failure_stops_new_work_but_running_tasks_finish() {
        let finished = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..20)
            .map(|i| {
                let finished = finished.clone();
                Task::new(format!("t{i}"), false, move || {
                    if i == 0 {
                        return Err::<(), String>("first".into());
                    }
                    thread::sleep(Duration::from_millis(40));
                    finished.fetch_add(1, Ordering::SeqCst);
                    if i == 1 { panic!("second") } else { Ok(()) }
                })
            })
            .collect();
        let outcomes = run(2, tasks, Result::is_err);
        let started = outcomes.iter().filter(|o| o.result.is_some()).count();
        assert!(started < 20, "scheduling continued after the failure");
        assert_eq!(finished.load(Ordering::SeqCst), started - 1);
        assert!(matches!(outcomes[0].result, Some(Ok(Err(_)))));
        assert!(
            outcomes[1]
                .result
                .as_ref()
                .unwrap()
                .as_ref()
                .is_err_and(|e| e.contains("second"))
        );
    }
}
