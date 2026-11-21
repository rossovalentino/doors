//! Phase-owned guest assertions. Runners and persistent acceptance share this
//! inventory so a removed scenario or weakened evidence marker cannot pass.
use crate::{emulator::Outcome, evidence::Run, phase::Phase};
use serde_json::{Value, json};

pub const NORMAL_PROFILE: &str = "profiles/hardware/qemu-diagnostic-x64.json";
pub const MINIMAL_PROFILE: &str = "profiles/hardware/qemu-minimal-x64.json";
pub const SMP2_PROFILE: &str = "profiles/hardware/qemu-smp2-x64.json";
pub const SMP4_PROFILE: &str = "profiles/hardware/qemu-smp4-x64.json";
pub struct Scenario {
    pub id: &'static str,
    pub mode: &'static str,
    pub profile: &'static str,
    pub expected: Outcome,
    pub markers: Vec<&'static str>,
}
pub fn cases(phase: &str) -> Result<Vec<Scenario>, String> {
    // P12/P13/P14 re-execute every inherited diagnostic guest scenario on
    // current source. Their new UI, Windows GUI and desktop scenarios run as
    // desktop acceptance suites (see `acceptance_suites`), not as boot modes.
    if phase == "P17" {
        return cases("P16");
    }
    if phase == "P16" {
        return cases("P15");
    }
    if phase == "P15" {
        return cases("P14");
    }
    if phase == "P14" {
        return cases("P13");
    }
    if phase == "P13" {
        return cases("P12");
    }
    if phase == "P12" {
        return cases("P11");
    }
    if phase == "P11" {
        return cases("P10");
    }
    if phase == "P10" {
        let mut inherited = cases("P09")?;
        inherited.extend(crate::windows::base_verify::cases());
        return Ok(inherited);
    }
    if phase == "P09" {
        let mut inherited = cases("P08")?;
        inherited.push(crate::windows::gate::case());
        return Ok(inherited);
    }
    if phase == "P08" {
        let mut inherited = cases("P07")?;
        inherited.extend(crate::services::scenarios::cases());
        return Ok(inherited);
    }
    if phase == "P07" {
        let mut inherited = cases("P06")?;
        inherited.extend(crate::persistence::scenarios::cases(&inherited[0].markers));
        return Ok(inherited);
    }
    if !matches!(phase, "P02" | "P03" | "P04" | "P05" | "P06") {
        return Err("guest gate for phase is not implemented".into());
    }
    let mut cases = [
        ("normal", Outcome::Passed, &["KERNEL_DIAGNOSTIC PASS"][..]),
        (
            "panic",
            Outcome::KernelPanic,
            &["intentional P02 panic"][..],
        ),
        (
            "bad-handoff",
            Outcome::GuestFailure,
            &["DOOR_KERNEL FAILURE: boot header"][..],
        ),
        (
            "hang",
            Outcome::Timeout,
            &["DOOR_KERNEL INTENTIONAL HANG"][..],
        ),
        (
            "bad-kernel",
            Outcome::StartupFailure,
            &["kernel ELF validation"][..],
        ),
        (
            "bad-initfs",
            Outcome::StartupFailure,
            &["initfs validation"][..],
        ),
    ]
    .into_iter()
    .map(|(id, expected, markers)| Scenario {
        id,
        mode: id,
        profile: NORMAL_PROFILE,
        expected,
        markers: markers.to_vec(),
    })
    .collect::<Vec<_>>();
    if matches!(phase, "P03" | "P04" | "P05" | "P06") {
        const NORMAL: &[&str] = &[
            "KERNEL_DIAGNOSTIC PASS",
            "DOOR_P03 MEMORY PASS",
            "DOOR_P03 CPU PASS",
            "DOOR_P03 TIMER PASS",
        ];
        cases[0].markers = NORMAL.to_vec();
        for (mode, markers) in [
            ("trap-ud", &["DOOR_TRAP vector=6 error=0x0"][..]),
            (
                "trap-double",
                &["DOOR_TRAP vector=8 error=0x0", "cr2=0xffffffffa2001ff8"][..],
            ),
            (
                "fault-readonly",
                &["DOOR_TRAP vector=14 error=0x3", "cr2=0xffffffffa2000000"][..],
            ),
            (
                "fault-nx",
                &["DOOR_TRAP vector=14 error=0x11", "cr2=0xffffffffa2000000"][..],
            ),
            (
                "fault-guard",
                &["DOOR_TRAP vector=14 error=0x2", "cr2=0xffffffffa2001000"][..],
            ),
        ] {
            cases.push(Scenario {
                id: mode,
                mode,
                profile: NORMAL_PROFILE,
                expected: Outcome::KernelPanic,
                markers: markers.to_vec(),
            });
        }
        cases.push(Scenario {
            id: "minimal",
            mode: "normal",
            profile: MINIMAL_PROFILE,
            expected: Outcome::Passed,
            markers: NORMAL.to_vec(),
        });
    }
    if matches!(phase, "P04" | "P05" | "P06") {
        const SINGLE: &[&str] = &[
            "KERNEL_DIAGNOSTIC PASS",
            "DOOR_P03 MEMORY PASS",
            "DOOR_P03 CPU PASS",
            "DOOR_P03 TIMER PASS",
            "DOOR_P04 TASKS PASS",
            "DOOR_P04 WAITS PASS",
            "DOOR_P04 SMP PASS discovered=1 online=1 degraded=0",
            "DOOR_P04 TLB PASS cpus=1",
        ];
        for case in &mut cases {
            if case.mode == "normal" {
                case.markers = SINGLE.to_vec();
            }
        }
        for (id, mode, profile, markers) in [
            (
                "normal-smp2",
                "normal",
                SMP2_PROFILE,
                &[
                    "KERNEL_DIAGNOSTIC PASS",
                    "DOOR_P03 MEMORY PASS",
                    "DOOR_P03 CPU PASS",
                    "DOOR_P03 TIMER PASS",
                    "DOOR_P04 TASKS PASS",
                    "DOOR_P04 WAITS PASS",
                    "DOOR_P04 SMP PASS discovered=2 online=2 degraded=0",
                    "DOOR_P04 TLB PASS cpus=2",
                ][..],
            ),
            (
                "normal-smp4",
                "normal",
                SMP4_PROFILE,
                &[
                    "KERNEL_DIAGNOSTIC PASS",
                    "DOOR_P03 MEMORY PASS",
                    "DOOR_P03 CPU PASS",
                    "DOOR_P03 TIMER PASS",
                    "DOOR_P04 TASKS PASS",
                    "DOOR_P04 WAITS PASS",
                    "DOOR_P04 SMP PASS discovered=4 online=4 degraded=0",
                    "DOOR_P04 TLB PASS cpus=4",
                ][..],
            ),
            (
                "smp-degraded",
                "smp-degraded",
                SMP2_PROFILE,
                &[
                    "KERNEL_DIAGNOSTIC PASS",
                    "DOOR_P03 MEMORY PASS",
                    "DOOR_P03 CPU PASS",
                    "DOOR_P03 TIMER PASS",
                    "DOOR_P04 TASKS PASS",
                    "DOOR_P04 WAITS PASS",
                    "DOOR_P04 SMP PASS discovered=2 online=1 degraded=1",
                    "DOOR_P04 TLB PASS cpus=1",
                ][..],
            ),
        ] {
            cases.push(Scenario {
                id,
                mode,
                profile,
                expected: Outcome::Passed,
                markers: markers.to_vec(),
            });
        }
    }
    if matches!(phase, "P05" | "P06") {
        for case in &mut cases {
            if case.expected == Outcome::Passed {
                case.markers.extend(p05_markers());
            }
        }
    }
    if phase == "P06" {
        for (mode, persistence) in [
            ("storage-write", "DOOR_P06 PERSIST PASS stage=write"),
            ("storage-reopen", "DOOR_P06 PERSIST PASS stage=reopen"),
        ] {
            let mut markers = cases[0].markers.clone();
            markers.extend([
                "DOOR_P06 DEVICE PASS",
                "DOOR_P06 BLOCK PASS",
                "DOOR_P06 STACK PASS",
                "DOOR_P06 FILESYSTEM PASS gpt=1 fat32=1 ext2=1 initfs=1 identity=1 coherent=1",
                "DOOR_P06 ERRORS PASS bounds=1 alignment=1 short_io=1 queue_full=1 reset=1 stale=1",
                "DOOR_P06 IO_FAILURE PASS short_write=1 flush=1 poisoned=1 remounted=1",
                persistence,
                "DOOR_P06 CLEANUP PASS dma_quiesced=1 pages_reclaimed=8",
            ]);
            cases.push(Scenario {
                id: mode,
                mode,
                profile: crate::storage::PROFILE,
                expected: Outcome::Passed,
                markers,
            });
        }
    }
    Ok(cases)
}

const P05_SUMMARIES: &[&str] = &[
    "DOOR_P05 NATIVE PASS processes=6 ring3=1 isolated=1",
    "DOOR_P05 IPC PASS authenticated=1 rights=1 shared=1",
    "DOOR_P05 FAULTS PASS kernel_read=-14 invalid_return=-257 survivor=44",
    "DOOR_P05 ABI PASS independent_probe=55",
    "DOOR_P05 CLEANUP PASS retired=6 leaked=0",
    "DOOR_P05 IPC_LIFETIME PASS cancellation=1 deadline=1",
];
const P05_USER_RECORDS: &[(&str, usize)] = &[
    ("DOOR_P05 CLIENT PASS", 0),
    ("DOOR_P05 SERVER PASS", 1),
    ("DOOR_P05 NEGATIVE PASS", 0),
    ("DOOR_P05 SURVIVOR PASS", 3),
    ("DOOR_P05 RAW_ABI PASS", 4),
    ("DOOR_P05 IPC_LIFETIME PASS", 0),
];
const P05_PROCESSES: &[&str] = &[
    "DOOR_P05 PROCESS role=1 exit=11",
    "DOOR_P05 PROCESS role=2 exit=22",
    "DOOR_P05 PROCESS role=3 exit=-14",
    "DOOR_P05 PROCESS role=4 exit=44",
    "DOOR_P05 PROCESS role=5 exit=55",
    "DOOR_P05 PROCESS role=6 exit=-257",
];
const P05_NUMERIC_MARKERS: &[&str] = &["DOOR_P05 MEMORY PASS", "DOOR_P05 STARTUP_ROLLBACK PASS"];
fn p05_markers() -> impl Iterator<Item = &'static str> {
    P05_SUMMARIES
        .iter()
        .copied()
        .chain(P05_USER_RECORDS.iter().map(|record| record.0))
        .chain(P05_PROCESSES.iter().copied())
        .chain(P05_NUMERIC_MARKERS.iter().copied())
}
/// P04 subsystem markers are unique complete line prefixes with a token boundary.
/// Earlier phases retain their original marker interpretation for historical evidence.
pub fn serial_matches(phase: &str, markers: &[&str], serial: &str) -> bool {
    if matches!(phase, "P11" | "P12" | "P13" | "P14" | "P15" | "P16" | "P17") {
        return serial_matches("P10", markers, serial);
    }
    if phase == "P10" {
        if markers.contains(&"WINDOWS_BASE_ADAPTER_PASS") {
            return crate::windows::base_verify::serial_matches(serial);
        }
        return serial_matches("P09", markers, serial);
    }
    if phase == "P09" {
        if markers.iter().any(|m| m.starts_with("DOOR_P09 ")) {
            return crate::windows::gate::serial_matches(serial);
        }
        return serial_matches("P08", markers, serial);
    }
    if phase == "P08" {
        let old: Vec<_> = markers
            .iter()
            .copied()
            .filter(|marker| !marker.starts_with("DOOR_P08 "))
            .collect();
        return serial_matches("P07", &old, serial)
            && crate::services::scenarios::serial_matches(markers, serial);
    }
    if phase == "P07" {
        let old: Vec<_> = markers
            .iter()
            .copied()
            .filter(|m| !m.starts_with("DOOR_P07 "))
            .collect();
        return serial_matches("P06", &old, serial)
            && crate::persistence::scenarios::serial_matches(markers, serial);
    }
    if phase == "P06"
        && markers.iter().any(|marker| marker.starts_with("DOOR_P06 "))
        && !crate::storage::serial_matches(markers, serial)
    {
        return false;
    }
    if matches!(phase, "P05" | "P06")
        && markers.iter().any(|marker| marker.starts_with("DOOR_P05 "))
        && !native_records_match(serial)
    {
        return false;
    }
    markers.iter().all(|marker| {
        if phase == "P06" && marker.starts_with("DOOR_P06 ") {
            true
        } else if matches!(phase, "P05" | "P06") && marker.starts_with("DOOR_P05 ") {
            // Native records are fully parsed together, including authenticated
            // PID attribution, distinct processes and exact per-role outcomes.
            true
        } else if matches!(phase, "P04" | "P05" | "P06") && marker.starts_with("DOOR_P04 ") {
            let prefix = marker.split(" PASS").next().unwrap_or(marker);
            let mut records = serial.lines().filter(|line| line.starts_with(prefix));
            let Some(line) = records.next() else {
                return false;
            };
            if records.next().is_some()
                || !(line == *marker
                    || line
                        .strip_prefix(marker)
                        .is_some_and(|tail| tail.starts_with(' ')))
            {
                return false;
            }
            let fields: &[&str] = if marker.starts_with("DOOR_P04 SMP ") {
                &["discovered=", "online=", "degraded="]
            } else if marker.starts_with("DOOR_P04 TLB ") {
                &["cpus="]
            } else {
                &[]
            };
            if fields.iter().any(|field| {
                line.split_ascii_whitespace()
                    .filter(|word| word.starts_with(field))
                    .count()
                    != 1
            }) {
                return false;
            }
            if marker.starts_with("DOOR_P04 TLB ") {
                let rounds = line
                    .split_ascii_whitespace()
                    .filter_map(|word| word.strip_prefix("rounds="))
                    .collect::<Vec<_>>();
                rounds.len() == 1 && rounds[0].parse::<u64>().is_ok_and(|n| n > 0)
            } else {
                true
            }
        } else {
            serial.contains(marker)
        }
    })
}

fn native_records_match(serial: &str) -> bool {
    let Some(memory) = numeric_record(
        serial,
        "DOOR_P05 MEMORY",
        &["allocated_pages=", "kernel_table_pages=", "copy_faults="],
    ) else {
        return false;
    };
    if memory[0] == 0 || !(1..=64).contains(&memory[1]) || memory[2] != 6 {
        return false;
    }
    if rollback_stages(serial).is_none_or(|stages| !(12..=26).contains(&stages) || stages % 2 != 0)
    {
        return false;
    }
    for marker in P05_SUMMARIES {
        let prefix = marker.split(" PASS").next().unwrap();
        let records = serial
            .lines()
            .filter(|line| {
                *line == prefix
                    || line
                        .strip_prefix(prefix)
                        .is_some_and(|tail| tail.starts_with(' '))
            })
            .collect::<Vec<_>>();
        if records != [*marker] {
            return false;
        }
    }
    let mut pids = [0u64; 6];
    let processes = serial
        .lines()
        .filter(|line| line.starts_with("DOOR_P05 PROCESS "))
        .collect::<Vec<_>>();
    if processes.len() != 6 {
        return false;
    }
    for (index, marker) in P05_PROCESSES.iter().enumerate() {
        let prefix = format!("{marker} pid=");
        let records = processes
            .iter()
            .filter_map(|line| line.strip_prefix(&prefix))
            .collect::<Vec<_>>();
        if records.len() != 1 {
            return false;
        }
        let Ok(pid) = records[0].parse::<u64>() else {
            return false;
        };
        if pid == 0 || pids[..index].contains(&pid) || pid.to_string() != records[0] {
            return false;
        }
        pids[index] = pid;
    }
    for (marker, role) in P05_USER_RECORDS {
        let expected = format!("DOOR_USER pid={} {marker}", pids[*role]);
        let matching = serial
            .lines()
            .filter(|line| line.starts_with("DOOR_USER pid=") && line.contains(*marker))
            .collect::<Vec<_>>();
        if matching != [expected.as_str()] {
            return false;
        }
    }
    true
}

fn numeric_record(serial: &str, name: &str, fields: &[&str]) -> Option<Vec<u64>> {
    let mut matching = serial.lines().filter(|line| {
        *line == name
            || line
                .strip_prefix(name)
                .is_some_and(|tail| tail.starts_with(' '))
    });
    let line = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    let prefix = format!("{name} PASS ");
    let words = line.strip_prefix(&prefix)?.split(' ').collect::<Vec<_>>();
    if words.len() != fields.len() {
        return None;
    }
    words
        .iter()
        .zip(fields)
        .map(|(word, field)| {
            let text = word.strip_prefix(field)?;
            let value = text.parse::<u64>().ok()?;
            (value.to_string() == text).then_some(value)
        })
        .collect()
}

pub fn rollback_stages(serial: &str) -> Option<u64> {
    let values = numeric_record(serial, "DOOR_P05 STARTUP_ROLLBACK", &["stages=", "leaks="])?;
    (values[1] == 0).then_some(values[0])
}

pub fn check_id(phase: &str) -> Result<&'static str, String> {
    match phase {
        "P02" => Ok("p02-independent-boot"),
        "P03" => Ok("p03-memory-cpu"),
        "P04" => Ok("p04-scheduler-smp"),
        "P05" => Ok("p05-protected-native"),
        "P06" => Ok("p06-storage-persistence"),
        "P07" => Ok("p07-journal-namespace"),
        "P08" => Ok("p08-native-services"),
        "P09" => Ok("p09-windows-x64"),
        "P10" => Ok("p10-windows-base"),
        "P11" => Ok("p11-native-window-regressions"),
        "P12" => Ok("p12-inherited-foundation-regressions"),
        "P13" => Ok("p13-inherited-foundation-regressions"),
        "P14" => Ok("p14-inherited-foundation-regressions"),
        "P15" => Ok("p15-inherited-foundation-regressions"),
        "P16" => Ok("p16-inherited-foundation-regressions"),
        "P17" => Ok("p17-inherited-foundation-regressions"),
        _ => Err("guest gate for phase is not implemented".into()),
    }
}

/// A specification-named acceptance suite that a phase adds on top of the
/// inherited diagnostic guest scenarios and desktop workflows. Later phases
/// inherit every earlier suite. The suite is the gate's contract; the runner is
/// supplied by the owning workstream when its scenario exists.
#[derive(Debug)]
pub struct AcceptanceSuite {
    /// Acceptance/requirement identifier from the master specification.
    pub id: &'static str,
    /// Phase that introduces the suite; every later phase inherits it.
    pub phase: Phase,
    /// `cargo xtask test <family> --suite <name>` (spec section 86).
    pub family: &'static str,
    pub name: &'static str,
    /// Check recorded by verify and required by the current phase gate.
    pub check: &'static str,
    /// Exact master-specification line references.
    pub spec: &'static [&'static str],
    /// Workstream expected to implement the runner (docs/design/p12-p13-plan-20261023.md).
    pub owner: &'static str,
    /// `project/requirements.json` records the suite qualifies.
    pub requirements: &'static [&'static str],
    pub summary: &'static str,
}

/// Runs one acceptance suite against the current production desktop image and
/// its paired test image, returning the suite's own evidence record.
pub type SuiteRunner = fn(&mut Run, &str, &str) -> Result<Value, String>;

const ACCEPTANCE: &[AcceptanceSuite] = &[
    AcceptanceSuite {
        id: "TEXT-01",
        phase: Phase::P12,
        family: "ui",
        name: "text-01",
        check: "p12-text-01",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4275 TEXT-01",
            "DOOR_OS_MASTER_SPECIFICATION.md:2647 P12 exit evidence",
            "DOOR_OS_MASTER_SPECIFICATION.md:1222 section 52",
            "DOOR_OS_MASTER_SPECIFICATION.md:1240 section 53",
        ],
        owner: "T Door Text, I input/IME and W5 clipboard; Notes as the TEXT-01 editor (stage 2)",
        requirements: &[
            "P12-TEXT",
            "P12-LAYOUTS",
            "P12-INPUT-IME",
            "P12-CLIPBOARD-DND",
            "P12-EDITOR",
        ],
        summary: "Multilingual editing, IME composition, clipboard and saved encoding in the real native editor",
    },
    AcceptanceSuite {
        id: "A11Y-01",
        phase: Phase::P12,
        family: "ui",
        name: "a11y-01",
        check: "p12-a11y-01",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4276 A11Y-01",
            "DOOR_OS_MASTER_SPECIFICATION.md:1337 section 58",
            "DOOR_OS_MASTER_SPECIFICATION.md:3878 section 113",
        ],
        owner: "U Door UI toolkit and accessibility",
        requirements: &["P12-A11Y", "P12-TOOLKIT", "P12-TEXT-SIZE"],
        summary: "Keyboard-only navigation, semantic tree and focus restoration without screenshot recognition",
    },
    AcceptanceSuite {
        id: "DISPLAY-01",
        phase: Phase::P12,
        family: "ui",
        name: "display-01",
        check: "p12-display-01",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4279 DISPLAY-01",
            "DOOR_OS_MASTER_SPECIFICATION.md:1319 section 57",
        ],
        owner: "W8 fractional DPI core (display_scale scenario)",
        requirements: &["P12-DPI"],
        summary: "Scale and output changes plus presentation failure keep coherent geometry, valid buffers and a recoverable fallback",
    },
    AcceptanceSuite {
        id: "MOTION-01",
        phase: Phase::P12,
        family: "ui",
        name: "motion-01",
        check: "p12-motion-01",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4278 MOTION-01",
            "DOOR_OS_MASTER_SPECIFICATION.md:3833 section 111 motion capture",
        ],
        owner: "W7 themes, reduced motion and transparency (assignment to be confirmed by the coordinator)",
        requirements: &["P12-THEMES"],
        summary: "Interrupted animations and reduced motion reach correct trajectories and final state with no stuck input or idle timers",
    },
    AcceptanceSuite {
        id: "COMPONENT-GALLERY",
        phase: Phase::P12,
        family: "ui",
        name: "component-gallery",
        check: "p12-component-gallery",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:2494 section 86 command",
            "DOOR_OS_MASTER_SPECIFICATION.md:1298 section 55",
            "DOOR_OS_MASTER_SPECIFICATION.md:3895 section 113",
            "DOOR_OS_MASTER_SPECIFICATION.md:4218 UI-A",
            "DOOR_OS_MASTER_SPECIFICATION.md:4219 UI-B",
        ],
        owner: "U Door UI toolkit and accessibility (desktop/component-gallery)",
        requirements: &["P12-GALLERY", "P12-TOOLKIT", "P12-CONTROLS", "P12-THEMES"],
        summary: "Deterministic gallery of every control and state with interaction traces and semantic trees",
    },
    AcceptanceSuite {
        id: "WIN-GUI F1",
        phase: Phase::P13,
        family: "compat",
        name: "win32-gui-f1",
        check: "p13-win32-gui-f1",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4220 WIN-GUI",
            "DOOR_OS_MASTER_SPECIFICATION.md:1366 GUI-E",
            "DOOR_OS_MASTER_SPECIFICATION.md:2493 section 86 command",
            "DOOR_OS_MASTER_SPECIFICATION.md:4661 section 136.3",
        ],
        owner: "P13-C fixtures, reference and guest integration",
        requirements: &["WIN-FIXTURES", "WIN-REFERENCE", "WIN-USER32", "WIN-GDI32"],
        summary: "F1 self-test creates windows, pumps messages, paints and takes real input; same binary as the Windows reference run",
    },
    AcceptanceSuite {
        id: "WIN-GUI F2",
        phase: Phase::P13,
        family: "compat",
        name: "win32-gui-f2",
        check: "p13-win32-gui-f2",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4220 WIN-GUI",
            "DOOR_OS_MASTER_SPECIFICATION.md:4272 GUI-02",
            "DOOR_OS_MASTER_SPECIFICATION.md:2648 P13 exit evidence",
        ],
        owner: "P13-C fixtures, reference and guest integration",
        requirements: &[
            "WIN-FIXTURES",
            "WIN-USER32",
            "WIN-GDI32",
            "WIN-COMDLG32",
            "WIN-KERNEL32",
        ],
        summary: "F2 Notepad-like editor (and RC variant) edits with real input, opens/saves through the dialog contract, relaunches and reopens exact bytes",
    },
    AcceptanceSuite {
        id: "WIN-GUI F3",
        phase: Phase::P13,
        family: "compat",
        name: "win32-gui-f3",
        check: "p13-win32-gui-f3",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4220 WIN-GUI",
            "DOOR_OS_MASTER_SPECIFICATION.md:4273 GUI-03",
        ],
        owner: "P13-C fixtures, reference and guest integration",
        requirements: &["WIN-FIXTURES", "WIN-USER32"],
        summary: "F3 hang fixture is contained (SendMessageTimeoutW, IsHungAppWindow, server close deadline) while the desktop stays usable",
    },
    AcceptanceSuite {
        id: "141.5",
        phase: Phase::P13,
        family: "compat",
        name: "scenario-141-5",
        check: "p13-scenario-141-5",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:5283 section 141.5",
            "DOOR_OS_MASTER_SPECIFICATION.md:4271 GUI-01",
        ],
        owner: "P13-C fixtures, reference and guest integration",
        requirements: &["WIN-USER32", "WIN-GDI32", "WIN-PROTOCOL", "WIN-LAUNCH"],
        summary: "Real Windows editor coexists with two native windows: move/overlap/resize/minimize/focus, multilingual text, clipboard, open/save/relaunch, hang one app, destroy during resize",
    },
    AcceptanceSuite {
        id: "141.6",
        phase: Phase::P13,
        family: "compat",
        name: "scenario-141-6",
        check: "p13-scenario-141-6",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:5287 section 141.6",
            "DOOR_OS_MASTER_SPECIFICATION.md:4273 GUI-03",
            "DOOR_OS_MASTER_SPECIFICATION.md:3723 section 107.3",
        ],
        owner: "P13-B user32 core with P13-C fixtures",
        requirements: &["WIN-USER32"],
        summary: "Synchronous SendMessage with permitted nested calls, posted ordering, destroyed destination and hung recipient with the supported timeout",
    },
    // P14 complete desktop and visual fidelity (owner decision 2026-10-26,
    // ADR 0020, docs/design/p14-plan-20261026.md). One family, `desktop`.
    AcceptanceSuite {
        id: "DESKTOP-A shell",
        phase: Phase::P14,
        family: "desktop",
        name: "desktop-a-shell",
        check: "p14-desktop-a-shell",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4221 DESKTOP-A",
            "DOOR_OS_MASTER_SPECIFICATION.md:1367 GUI-F",
            "DOOR_OS_MASTER_SPECIFICATION.md:1383 section 60",
            "DOOR_OS_MASTER_SPECIFICATION.md:2649 P14 exit evidence",
        ],
        owner: "WS-A shell extraction, registration and notification services",
        requirements: &["P14-SHELL", "P14-REGISTRY", "P14-NOTIFY", "DESKTOP-A"],
        summary: "Restartable shell process: taskbar from real top-level windows (native and Win32), registration-based launcher, tray and notifications; every visible control performs a real action",
    },
    AcceptanceSuite {
        id: "RECOVER-01",
        phase: Phase::P14,
        family: "desktop",
        name: "recover-01",
        check: "p14-recover-01",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4296 RECOVER-01",
            "DOOR_OS_MASTER_SPECIFICATION.md:1367 GUI-F",
            "DOOR_OS_MASTER_SPECIFICATION.md:3899 section 114",
        ],
        owner: "WS-A shell extraction, registration and notification services",
        requirements: &["P14-RECOVERY"],
        summary: "Kill the shell: native and Win32 applications keep running and the restarted shell rebuilds its state from the authority; no app restart hides loss",
    },
    AcceptanceSuite {
        id: "DESKTOP-A files",
        phase: Phase::P14,
        family: "desktop",
        name: "desktop-a-files",
        check: "p14-desktop-a-files",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4221 DESKTOP-A",
            "DOOR_OS_MASTER_SPECIFICATION.md:1399 section 61",
            "DOOR_OS_MASTER_SPECIFICATION.md:2649 P14 exit evidence",
        ],
        owner: "WS-B service-backed applications (file operations)",
        requirements: &["P14-FILEOPS", "DESKTOP-A"],
        summary: "Copy/move engine with progress, cancel and conflicts, durable recycle/restore and change notification, audited on raw disk across a reboot",
    },
    AcceptanceSuite {
        id: "DESKTOP-A apps",
        phase: Phase::P14,
        family: "desktop",
        name: "desktop-a-apps",
        check: "p14-desktop-a-apps",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4221 DESKTOP-A",
            "DOOR_OS_MASTER_SPECIFICATION.md:1415 section 62",
            "DOOR_OS_MASTER_SPECIFICATION.md:1431 section 63",
        ],
        owner: "WS-B service-backed applications (settings registry, terminal)",
        requirements: &["P14-SETTINGS", "P14-TERMINAL", "DESKTOP-A"],
        summary: "Typed settings registry propagates to running apps and survives a corrupt configuration; terminal selection, resize and backpressure",
    },
    AcceptanceSuite {
        id: "SEC-02 trusted",
        phase: Phase::P14,
        family: "desktop",
        name: "sec-02-trusted",
        check: "p14-sec-02-trusted",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4294 SEC-02",
            "DOOR_OS_MASTER_SPECIFICATION.md:2649 P14 trusted logon UI",
        ],
        owner: "WS-C sign-in, entropy and credential store",
        requirements: &["P14-LOGON"],
        summary: "Win+L locks; a wrong then the right password/PIN (Argon2id, kernel-seeded salt) with throttling; capture and input injection by ordinary apps are denied while locked",
    },
    AcceptanceSuite {
        id: "VIS-01",
        phase: Phase::P14,
        family: "desktop",
        name: "vis-01",
        check: "p14-vis-01",
        spec: &[
            "DOOR_OS_MASTER_SPECIFICATION.md:4277 VIS-01",
            "DOOR_OS_MASTER_SPECIFICATION.md:4222 DESKTOP-B",
            "DOOR_OS_MASTER_SPECIFICATION.md:1368 GUI-G",
            "DOOR_OS_MASTER_SPECIFICATION.md:3815 section 111",
            "DOOR_OS_MASTER_SPECIFICATION.md:3847 section 112",
            "DOOR_OS_MASTER_SPECIFICATION.md:5255 section 140.6",
        ],
        owner: "WS-C reference profile, tokens and fidelity harness",
        requirements: &[
            "P14-REFERENCE",
            "P14-TOKENS",
            "P14-MARK",
            "P14-VIS",
            "DESKTOP-B",
        ],
        summary: "Frozen shell surface set compared against profiles/windows-reference in five dimensions (geometry, colour, typography, iconography, motion) with frozen thresholds and explicit deviations",
    },
];

/// Implemented acceptance runners, keyed by suite name. An owning workstream
/// registers its runner here (through the gate owner) once the named scenario
/// exists. A suite without an entry is "not implemented" and can never pass.
const RUNNERS: &[(&str, SuiteRunner)] = &[
    ("component-gallery", crate::desktop::component_gallery_suite),
    ("a11y-01", crate::desktop::a11y_01_suite),
    ("text-01", crate::desktop::text_01_suite),
    ("display-01", crate::desktop::display_01_suite),
    ("motion-01", crate::desktop::motion_01_suite),
    ("win32-gui-f1", crate::desktop::win32_gui_f1_suite),
    ("win32-gui-f2", crate::desktop::win32_gui_f2_suite),
    ("win32-gui-f3", crate::desktop::win32_gui_f3_suite),
    ("scenario-141-5", crate::desktop::scenario_141_5_suite),
    ("scenario-141-6", crate::desktop::scenario_141_6_suite),
    ("desktop-a-shell", crate::desktop::desktop_a_shell_suite),
    ("recover-01", crate::desktop::recover_01_suite),
    ("desktop-a-files", crate::desktop::desktop_a_files_suite),
    ("desktop-a-apps", crate::desktop::desktop_a_apps_suite),
    ("sec-02-trusted", crate::desktop::sec_02_trusted_suite),
    ("vis-01", crate::fidelity::vis_01_suite),
];

/// Every acceptance suite the selected phase must pass, cumulatively.
pub fn acceptance_suites(phase: &str) -> Result<Vec<&'static AcceptanceSuite>, String> {
    let selected = Phase::parse(phase)?;
    Ok(ACCEPTANCE
        .iter()
        .filter(|suite| suite.phase <= selected)
        .collect())
}

/// Resolves `cargo xtask test <family> --suite <name>`.
pub fn acceptance_suite(family: &str, name: &str) -> Result<&'static AcceptanceSuite, String> {
    ACCEPTANCE
        .iter()
        .find(|suite| suite.family == family && suite.name == name)
        .ok_or_else(|| {
            let known: Vec<_> = ACCEPTANCE
                .iter()
                .filter(|suite| suite.family == family)
                .map(|suite| suite.name)
                .collect();
            format!(
                "unknown {family} acceptance suite {name}; declared suites: {}",
                known.join(", ")
            )
        })
}

fn describe(suite: &AcceptanceSuite) -> String {
    format!(
        "{} `cargo xtask test {} --suite {}` [{:?}; {}; owner: {}; requirements: {}]",
        suite.id,
        suite.family,
        suite.name,
        suite.phase,
        suite.spec.join(", "),
        suite.owner,
        suite.requirements.join(", ")
    )
}

/// The registered runner, or a clear "suite not implemented" failure.
pub fn runner(suite: &AcceptanceSuite) -> Result<SuiteRunner, String> {
    RUNNERS
        .iter()
        .find(|(name, _)| *name == suite.name)
        .map(|(_, runner)| *runner)
        .ok_or_else(|| {
            format!(
                "acceptance suite not implemented: {} — {}",
                describe(suite),
                suite.summary
            )
        })
}

/// Fails before any build or guest execution when a selected-phase suite has no
/// runner, naming every missing suite so the gate can never silently pass.
pub fn acceptance_preflight(phase: &str) -> Result<(), String> {
    let suites = acceptance_suites(phase)?;
    let missing: Vec<_> = suites
        .iter()
        .copied()
        .filter(|suite| runner(suite).is_err())
        .map(describe)
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{phase} acceptance suite not implemented ({} of {} declared): {}. Verification cannot pass until each named scenario has a registered runner",
        missing.len(),
        suites.len(),
        missing.join("; ")
    ))
}

/// The evidence check recorded for one executed suite.
/// The reference comparison a suite runner reports in its result
/// (`reference_comparison`), or `not_applicable`.
pub fn reference_comparison(result: &Value) -> Value {
    match &result["reference_comparison"] {
        Value::Null => json!("not_applicable"),
        value => value.clone(),
    }
}

pub fn acceptance_record(suite: &AcceptanceSuite, selected: &str, result: Value) -> Value {
    json!({"suite":suite.name,"family":suite.family,"requirement":suite.id,
        "introduced_by":format!("{:?}",suite.phase),"selected_phase":selected,
        "spec":suite.spec,"requirements":suite.requirements,"owner":suite.owner,
        "summary":suite.summary,"result":result})
}

/// A current P12/P13 report must carry a passed check for every cumulative
/// suite, bound to the declared suite identity and a non-empty runner result.
pub fn validate_acceptance(report: &Value, phase: &str) -> Result<(), String> {
    let checks = report["checks"]
        .as_array()
        .ok_or("acceptance checks missing")?;
    for suite in acceptance_suites(phase)? {
        let passed = checks.iter().any(|check| {
            check["id"] == suite.check
                && check["outcome"] == "passed"
                && check["details"]["suite"] == suite.name
                && check["details"]["requirement"] == suite.id
                && check["details"]["selected_phase"] == phase
                && check["details"]["result"]
                    .as_object()
                    .is_some_and(|result| !result.is_empty())
        });
        if !passed {
            return Err(format!(
                "{phase} acceptance suite {} has no passing evidence; {}",
                suite.name,
                runner(suite).err().unwrap_or_else(|| describe(suite))
            ));
        }
    }
    Ok(())
}

/// Runs one existing desktop scenario against the current production image.
pub type DesktopRunner = fn(&mut Run, &str) -> Result<(), String>;

/// Check recorded once per executed P12 desktop scenario.
pub const DESKTOP_SCENARIO_CHECK: &str = "p12-desktop-scenario";

/// A standalone desktop scenario (`cargo xtask test <name> --image <image>`) that
/// the P12 gate adds after the P11 desktop workflows of `desktop::verify_all`.
pub struct DesktopScenario {
    pub name: &'static str,
    /// Timing-sensitive (cursor-latency limits, recorded drag/motion timings,
    /// automatic host backend selection): the guest pool runs it with no
    /// other guest in flight.
    pub exclusive: bool,
    /// The `desktop::<function>` entry point (checked by the coverage test).
    pub function: &'static str,
    pub run: DesktopRunner,
}

/// Desktop entry points that `desktop::verify_all` (the P11 desktop gate, run by
/// every P12/P13 verify first) already executes.
#[cfg(test)]
const DESKTOP_GATE_FUNCTIONS: &[&str] = &[
    "recovery_test",
    "wide_test",
    "resolution_test",
    "resolution_test_mode",
    "native_default_test",
];

/// P18 desktop entry points (`cargo xtask test p18-*`). P18 is not in the
/// phase ledger yet, so no gate runs them; they join a P18 list when it is.
#[cfg(test)]
const P18_PENDING_FUNCTIONS: &[&str] = &[
    "p18_mem_test",
    "p18_crt_test",
    "p18_dotnet_test",
    "p18_pwsh_test",
    "p18_pwsh_b_test",
    "p18_electron_test",
];

/// Every other existing desktop scenario, software and accelerated variants alike.
const DESKTOP_SCENARIOS: &[DesktopScenario] = &[
    DesktopScenario {
        name: "desktop-notes",
        exclusive: false,
        function: "notes_test",
        run: |run, image| crate::desktop::notes_test(run, image, false),
    },
    DesktopScenario {
        name: "desktop-notes-accelerated",
        exclusive: false,
        function: "notes_test",
        run: |run, image| crate::desktop::notes_test(run, image, true),
    },
    DesktopScenario {
        name: "desktop-files",
        exclusive: false,
        function: "file_mutation_test",
        run: crate::desktop::file_mutation_test,
    },
    DesktopScenario {
        name: "desktop-files-accelerated",
        exclusive: false,
        function: "file_mutation_accelerated_test",
        run: crate::desktop::file_mutation_accelerated_test,
    },
    DesktopScenario {
        name: "desktop-remove",
        exclusive: false,
        function: "file_deletion_test",
        run: |run, image| crate::desktop::file_deletion_test(run, image, false),
    },
    DesktopScenario {
        name: "desktop-remove-accelerated",
        exclusive: false,
        function: "file_deletion_test",
        run: |run, image| crate::desktop::file_deletion_test(run, image, true),
    },
    DesktopScenario {
        name: "file-drag",
        exclusive: false,
        function: "file_drag_test",
        run: crate::desktop::file_drag_test,
    },
    DesktopScenario {
        name: "desktop-shell",
        exclusive: true,
        function: "shell_test",
        run: crate::desktop::shell_test,
    },
    DesktopScenario {
        name: "desktop-accelerated",
        exclusive: true,
        function: "accelerated_test",
        run: crate::desktop::accelerated_test,
    },
    DesktopScenario {
        name: "desktop-default",
        exclusive: true,
        function: "interactive_test",
        run: |run, image| crate::desktop::interactive_test(run, image, false),
    },
    DesktopScenario {
        name: "desktop-software",
        exclusive: true,
        function: "interactive_test",
        run: |run, image| crate::desktop::interactive_test(run, image, true),
    },
    DesktopScenario {
        name: "desktop-motion",
        exclusive: true,
        function: "motion_test",
        run: crate::desktop::motion_test,
    },
    DesktopScenario {
        name: "explorer",
        exclusive: false,
        function: "explorer_modern_test",
        run: crate::desktop::explorer_modern_test,
    },
    DesktopScenario {
        name: "multiwindow",
        exclusive: false,
        function: "multiwindow_test",
        run: crate::desktop::multiwindow_test,
    },
    DesktopScenario {
        name: "multiwindow-pressure",
        exclusive: false,
        function: "multiwindow_pressure_test",
        run: crate::desktop::multiwindow_pressure_test,
    },
    DesktopScenario {
        name: "monitor",
        exclusive: false,
        function: "monitor_test",
        run: crate::desktop::monitor_test,
    },
    DesktopScenario {
        name: "preview",
        exclusive: false,
        function: "preview_test",
        run: crate::desktop::preview_test,
    },
    DesktopScenario {
        name: "terminal",
        exclusive: false,
        function: "terminal_test",
        run: crate::desktop::terminal_test,
    },
    DesktopScenario {
        name: "settings-reference",
        exclusive: false,
        function: "settings_reference_test",
        run: crate::desktop::settings_reference_test,
    },
];

/// The desktop scenarios a phase's gate runs in addition to `desktop::verify_all`.
/// Desktop scenarios that P14 adds after every P12/P13 one.
const P14_DESKTOP_SCENARIOS: &[DesktopScenario] = &[
    DesktopScenario {
        name: "p14-settings",
        exclusive: false,
        function: "p14_settings_test",
        run: crate::desktop::p14_settings_test,
    },
    DesktopScenario {
        name: "p14-inbox",
        exclusive: false,
        function: "p14_inbox_test",
        run: crate::desktop::p14_inbox_test,
    },
];

/// Desktop scenarios P15 adds: the network and sound shell surfaces and
/// Settings pages (with the wav-captured Test tone), CONNECT-A's Windows
/// network programs against host endpoints, SOUND-A's winmm program and
/// its capture half, MIC-A (both exclusive: they measure real-time
/// playback, recording and timer periods).
const P15_DESKTOP_SCENARIOS: &[DesktopScenario] = &[
    DesktopScenario {
        name: "p15-desktop",
        exclusive: false,
        function: "p15_desktop_test",
        run: crate::desktop::p15_desktop_test,
    },
    DesktopScenario {
        name: "p15-connect",
        exclusive: false,
        function: "p15_connect_test",
        run: crate::desktop::p15_connect_test,
    },
    DesktopScenario {
        name: "p15-sound",
        exclusive: true,
        function: "p15_sound_test",
        run: crate::desktop::p15_sound_test,
    },
    DesktopScenario {
        name: "p15-capture",
        exclusive: true,
        function: "p15_capture_test",
        run: crate::desktop::p15_capture_test,
    },
    DesktopScenario {
        name: "p15-dsound",
        exclusive: true,
        function: "p15_dsound_test",
        run: crate::desktop::p15_dsound_test,
    },
    DesktopScenario {
        name: "p15-midi",
        exclusive: true,
        function: "p15_midi_test",
        run: crate::desktop::p15_midi_test,
    },
    DesktopScenario {
        name: "p15-wasapi",
        exclusive: true,
        function: "p15_wasapi_test",
        run: crate::desktop::p15_wasapi_test,
    },
    // A boringtun peer and its UDP port on this host.
    DesktopScenario {
        name: "p15-vpn",
        exclusive: true,
        function: "p15_vpn_test",
        run: crate::desktop::p15_vpn_test,
    },
];
/// Desktop scenarios P16 adds: cmd.exe in Terminal.
const P16_DESKTOP_SCENARIOS: &[DesktopScenario] = &[
    DesktopScenario {
        name: "p16-cmd",
        exclusive: false,
        function: "p16_cmd_test",
        run: crate::desktop::p16_cmd_test,
    },
    DesktopScenario {
        name: "p16-com",
        exclusive: false,
        function: "p16_com_test",
        run: crate::desktop::p16_com_test,
    },
    DesktopScenario {
        name: "p16-oleaut",
        exclusive: false,
        function: "p16_oleaut_test",
        run: crate::desktop::p16_oleaut_test,
    },
    DesktopScenario {
        name: "p16-tsched",
        exclusive: false,
        function: "p16_tsched_test",
        run: crate::desktop::p16_tsched_test,
    },
    DesktopScenario {
        name: "p16-crypt",
        exclusive: false,
        function: "p16_crypt_test",
        run: crate::desktop::p16_crypt_test,
    },
    DesktopScenario {
        name: "p16-trust",
        exclusive: false,
        function: "p16_trust_test",
        run: crate::desktop::p16_trust_test,
    },
    DesktopScenario {
        name: "p16-paths",
        exclusive: false,
        function: "p16_paths_test",
        run: crate::desktop::p16_paths_test,
    },
    DesktopScenario {
        name: "p16-comoop",
        exclusive: false,
        function: "p16_comoop_test",
        run: crate::desktop::p16_comoop_test,
    },
    DesktopScenario {
        name: "p16-shell",
        exclusive: false,
        function: "p16_shell_test",
        run: crate::desktop::p16_shell_test,
    },
    DesktopScenario {
        name: "p16-install",
        exclusive: false,
        function: "p16_install_test",
        run: crate::desktop::p16_install_test,
    },
    DesktopScenario {
        name: "p16-accounts",
        exclusive: false,
        function: "p16_accounts_test",
        run: crate::desktop::p16_accounts_test,
    },
    DesktopScenario {
        name: "p16-scm",
        exclusive: false,
        function: "p16_scm_test",
        run: crate::desktop::p16_scm_test,
    },
    DesktopScenario {
        name: "p16-security",
        exclusive: false,
        function: "p16_security_test",
        run: crate::desktop::p16_security_test,
    },
];
/// Desktop scenarios P17 adds: 32-bit programs under WOW64 (X86-A, X86-B,
/// X86-C).
const P17_DESKTOP_SCENARIOS: &[DesktopScenario] = &[
    DesktopScenario {
        name: "p17-x86",
        exclusive: false,
        function: "p17_x86_test",
        run: crate::desktop::p17_x86_test,
    },
    DesktopScenario {
        name: "p17-wow64",
        exclusive: false,
        function: "p17_wow64_test",
        run: crate::desktop::p17_wow64_test,
    },
];

pub fn desktop_scenarios(phase: &str) -> Result<Vec<&'static DesktopScenario>, String> {
    let phase = Phase::parse(phase)?;
    let mut list: Vec<_> = if phase >= Phase::P12 {
        DESKTOP_SCENARIOS.iter().collect()
    } else {
        Vec::new()
    };
    if phase >= Phase::P14 {
        list.extend(P14_DESKTOP_SCENARIOS);
    }
    if phase >= Phase::P15 {
        list.extend(P15_DESKTOP_SCENARIOS);
    }
    if phase >= Phase::P16 {
        list.extend(P16_DESKTOP_SCENARIOS);
    }
    if phase >= Phase::P17 {
        list.extend(P17_DESKTOP_SCENARIOS);
    }
    Ok(list)
}

/// Runs each scenario in its own child evidence run (as `boot::verify` does for
/// guest cases), so its check identities never collide with the P11 desktop
/// validator's; the parent records one check bound to the child report digest.
pub fn run_desktop_scenarios(run: &mut Run, phase: &str, image: &str) -> Result<(), String> {
    for scenario in desktop_scenarios(phase)? {
        let started = std::time::Instant::now();
        let mut child = Run::new(run.root.clone())?;
        let result = (scenario.run)(&mut child, image);
        let finished = child.finish(result.as_ref().err().map(String::as_str));
        run.record_timing(
            "desktop-scenario",
            scenario.name,
            started.elapsed(),
            if finished.is_ok() && result.is_ok() {
                "passed"
            } else {
                "failed"
            },
        );
        finished?;
        let report = child.directory.join("report.json");
        let path = crate::evidence::relative(&run.root, &report)?;
        if let Err(error) = result {
            return Err(format!(
                "{phase} desktop scenario {} failed (report {path}): {error}",
                scenario.name
            ));
        }
        let checks: Vec<_> = child.checks().iter().map(|c| c["id"].clone()).collect();
        run.check(
            DESKTOP_SCENARIO_CHECK,
            scenario_record(scenario, phase, image, checks, &path, &report)?,
        );
    }
    Ok(())
}

/// The parent check of one passed desktop scenario, bound to its child report.
fn scenario_record(
    scenario: &DesktopScenario,
    phase: &str,
    image: &str,
    checks: Vec<Value>,
    path: &str,
    report: &std::path::Path,
) -> Result<Value, String> {
    Ok(
        json!({"scenario":scenario.name,"function":scenario.function,
        "command":format!("cargo xtask test {} --image <production-image.json>",scenario.name),
        "introduced_by":"P12","selected_phase":phase,"image":image,"checks":checks,
        "report":{"path":path,"sha256":crate::evidence::file_hash(report)?}}),
    )
}

/// Acceptance suites that run in an exclusive guest slot: those measuring
/// timing (guest animation clocks, idle frame counts, host-timed motion
/// sequences) and RECOVER-01, which kills the shell and races its restart
/// like the window-server recovery gate step.
pub fn exclusive_suite(name: &str) -> bool {
    matches!(name, "motion-01" | "vis-01" | "recover-01")
}

/// What one guest task of `run_guest_gate` leaves for the parent run.
enum GuestWork {
    /// A desktop gate step or acceptance suite: the forked parent ledger it
    /// recorded into, and its result.
    Ledger(Box<Run>, Result<Value, String>),
    /// A desktop scenario: its own finished child run.
    Child {
        report: std::path::PathBuf,
        checks: Vec<Value>,
        finished: Result<(), String>,
        result: Result<(), String>,
    },
    /// The child run could not be created.
    Unstarted(String),
}
impl GuestWork {
    fn failed(&self) -> bool {
        match self {
            Self::Ledger(_, result) => result.is_err(),
            Self::Child {
                finished, result, ..
            } => finished.is_err() || result.is_err(),
            Self::Unstarted(_) => true,
        }
    }
}

/// The P11 desktop gate steps, then (P12 and later) every desktop scenario
/// and acceptance suite, on `jobs` guest workers (`parallel::run`). Records
/// land in the parent in declared order with the same identities as the
/// sequential gate: gate steps and suites through forked ledgers of `run`
/// (same directory and run identity), scenarios in their own child runs.
/// `desktop::validate` sees exactly the parent records of the sequential
/// gate at the same point. Every failure is reported with its report path.
///
/// `only` (the `test guest-batch` developer command) keeps the named gate
/// steps, scenarios and suites; the completeness validators (desktop gate,
/// acceptance) then run only when nothing of theirs was left out.
pub fn run_guest_gate(
    run: &mut Run,
    phase: &str,
    image: &str,
    test_image: &str,
    jobs: usize,
    only: Option<&[String]>,
) -> Result<(), String> {
    let selected = Phase::parse(phase)?;
    let wanted = |name: &str| only.is_none_or(|names| names.iter().any(|n| n == name));
    let all_steps = crate::desktop::GATE_STEPS;
    let steps: Vec<&crate::desktop::GateStep> =
        all_steps.iter().filter(|s| wanted(s.name)).collect();
    let all_desktop = if selected >= Phase::P12 {
        desktop_scenarios(phase)?
    } else {
        Vec::new()
    };
    let desktop: Vec<&DesktopScenario> = all_desktop
        .iter()
        .copied()
        .filter(|s| wanted(s.name))
        .collect();
    let all_suites = if selected >= Phase::P12 {
        acceptance_suites(phase)?
    } else {
        Vec::new()
    };
    let suites: Vec<&AcceptanceSuite> = all_suites
        .iter()
        .copied()
        .filter(|s| wanted(s.name))
        .collect();
    if let Some(names) = only {
        for name in names {
            if !all_steps.iter().any(|s| s.name == name)
                && !all_desktop.iter().any(|s| s.name == name)
                && !all_suites.iter().any(|s| s.name == name)
            {
                return Err(format!(
                    "{name} is not a {phase} desktop gate step, desktop scenario or acceptance suite"
                ));
            }
        }
    }
    let complete_gate = steps.len() == all_steps.len();
    let complete_suites = suites.len() == all_suites.len();
    let runners = suites
        .iter()
        .map(|suite| runner(suite))
        .collect::<Result<Vec<_>, _>>()?;
    let parent_report = crate::evidence::relative(&run.root, &run.directory.join("report.json"))?;
    // The Windows GUI fixture image several suites boot is staged once,
    // before any of them starts (sequentially the first suite staged it).
    let stage = if suites
        .iter()
        .any(|suite| crate::desktop::uses_win32_stage(suite.name))
    {
        let mut ledger = run.fork_ledger();
        let started = std::time::Instant::now();
        let staged = crate::desktop::stage_win32(&mut ledger, image);
        run.record_timing(
            "build",
            "Windows GUI fixture stage",
            started.elapsed(),
            if staged.is_ok() { "passed" } else { "failed" },
        );
        if let Err(error) = staged {
            run.absorb(ledger);
            return Err(format!("{phase} Windows GUI fixture stage failed: {error}"));
        }
        Some(ledger)
    } else {
        None
    };
    let root = run.root.clone();
    let mut tasks = Vec::new();
    for step in &steps {
        let step = *step;
        let mut ledger = run.fork_ledger();
        tasks.push(crate::parallel::Task::new(
            format!("desktop {}", step.name),
            step.exclusive,
            move || {
                let result = (step.run)(&mut ledger, image, test_image).map(|()| Value::Null);
                GuestWork::Ledger(Box::new(ledger), result)
            },
        ));
    }
    for scenario in &desktop {
        let root = root.clone();
        let scenario = *scenario;
        tasks.push(crate::parallel::Task::new(
            scenario.name,
            scenario.exclusive,
            move || {
                let mut child = match Run::new(root) {
                    Ok(child) => child,
                    Err(error) => return GuestWork::Unstarted(error),
                };
                let result = (scenario.run)(&mut child, image);
                let finished = child.finish(result.as_ref().err().map(String::as_str));
                GuestWork::Child {
                    report: child.directory.join("report.json"),
                    checks: child.checks().iter().map(|c| c["id"].clone()).collect(),
                    finished,
                    result,
                }
            },
        ));
    }
    for (suite, runner) in suites.iter().zip(runners) {
        let mut ledger = run.fork_ledger();
        tasks.push(crate::parallel::Task::new(
            format!("suite {}", suite.name),
            exclusive_suite(suite.name),
            move || {
                let result = runner(&mut ledger, image, test_image);
                GuestWork::Ledger(Box::new(ledger), result)
            },
        ));
    }
    let mut outcomes = crate::parallel::run(jobs, tasks, GuestWork::failed).into_iter();
    let mut failures = Vec::new();
    let mut not_started = Vec::new();
    // Desktop gate steps: absorbed in order, then the desktop validator.
    let mut gate_passed = true;
    for (step, outcome) in steps.iter().zip(outcomes.by_ref()) {
        let state = match outcome.result {
            None => {
                not_started.push(outcome.name);
                gate_passed = false;
                "not_started"
            }
            Some(Err(panic)) => {
                failures.push(format!(
                    "desktop gate step {} (report {parent_report}): {panic}",
                    step.name
                ));
                gate_passed = false;
                "panicked"
            }
            Some(Ok(GuestWork::Ledger(ledger, result))) => {
                run.absorb(*ledger);
                if let Err(error) = result {
                    failures.push(format!(
                        "desktop gate step {} failed (report {parent_report}): {error}",
                        step.name
                    ));
                    gate_passed = false;
                    "failed"
                } else {
                    "passed"
                }
            }
            Some(Ok(_)) => unreachable!("gate steps record into forked ledgers"),
        };
        run.record_timing("desktop-gate-step", step.name, outcome.elapsed, state);
    }
    if gate_passed
        && complete_gate
        && let Err(error) = crate::desktop::validate(
            &run.root,
            &json!({"id":run.id,"checks":run.checks(),"commands":run.commands(),"source_snapshot":run.build_inputs()}),
        )
    {
        failures.push(format!(
            "desktop gate validation (report {parent_report}): {error}"
        ));
    }
    // Desktop scenarios: one parent check per passed child run.
    for (scenario, outcome) in desktop.iter().zip(outcomes.by_ref()) {
        let state = match outcome.result {
            None => {
                not_started.push(outcome.name);
                "not_started"
            }
            Some(Err(panic)) => {
                failures.push(format!(
                    "{phase} desktop scenario {}: {panic}",
                    scenario.name
                ));
                "panicked"
            }
            Some(Ok(GuestWork::Unstarted(error))) => {
                failures.push(format!(
                    "{phase} desktop scenario {} could not start: {error}",
                    scenario.name
                ));
                "failed"
            }
            Some(Ok(GuestWork::Child {
                report,
                checks,
                finished,
                result,
            })) => {
                let path = crate::evidence::relative(&run.root, &report)
                    .unwrap_or_else(|_| report.display().to_string());
                match (finished, result) {
                    (Err(error), _) => {
                        failures.push(format!(
                            "{phase} desktop scenario {} evidence (report {path}): {error}",
                            scenario.name
                        ));
                        "failed"
                    }
                    (Ok(()), Err(error)) => {
                        failures.push(format!(
                            "{phase} desktop scenario {} failed (report {path}): {error}",
                            scenario.name
                        ));
                        "failed"
                    }
                    (Ok(()), Ok(())) => {
                        match scenario_record(scenario, phase, image, checks, &path, &report) {
                            Ok(record) => {
                                run.check(DESKTOP_SCENARIO_CHECK, record);
                                "passed"
                            }
                            Err(error) => {
                                failures.push(format!(
                                    "{phase} desktop scenario {} report (report {path}): {error}",
                                    scenario.name
                                ));
                                "failed"
                            }
                        }
                    }
                }
            }
            Some(Ok(GuestWork::Ledger(..))) => unreachable!("scenarios use child runs"),
        };
        run.record_timing("desktop-scenario", scenario.name, outcome.elapsed, state);
    }
    // Acceptance suites: the shared stage, then each suite's ledger in order.
    if let Some(ledger) = stage {
        run.absorb(ledger);
    }
    for (suite, outcome) in suites.iter().zip(outcomes.by_ref()) {
        let state = match outcome.result {
            None => {
                not_started.push(outcome.name);
                "not_started"
            }
            Some(Err(panic)) => {
                failures.push(format!(
                    "{phase} acceptance suite {} (report {parent_report}): {panic}",
                    suite.name
                ));
                "panicked"
            }
            Some(Ok(GuestWork::Ledger(ledger, result))) => {
                run.absorb(*ledger);
                match result {
                    Ok(result) => {
                        run.record_suite(suite.name, reference_comparison(&result));
                        run.check(suite.check, acceptance_record(suite, phase, result));
                        "passed"
                    }
                    Err(error) => {
                        failures.push(format!(
                            "{phase} acceptance suite {} failed (report {parent_report}): {error}",
                            suite.name
                        ));
                        "failed"
                    }
                }
            }
            Some(Ok(_)) => unreachable!("suites record into forked ledgers"),
        };
        run.record_timing("acceptance-suite", suite.name, outcome.elapsed, state);
    }
    if failures.is_empty() && not_started.is_empty() {
        if selected >= Phase::P12 && complete_suites {
            validate_acceptance(&json!({"checks":run.checks()}), phase)?;
        }
        return Ok(());
    }
    Err(format!(
        "{} guest task(s) failed{}: {}",
        failures.len(),
        if not_started.is_empty() {
            String::new()
        } else {
            format!(
                "; {} not started after the first failure ({})",
                not_started.len(),
                not_started.join(", ")
            )
        },
        failures.join("; ")
    ))
}

/// A current P12/P13 report needs exactly one passed check per declared desktop
/// scenario, bound to a successful child report with unchanged bytes.
pub fn validate_desktop_scenarios(
    root: &std::path::Path,
    report: &Value,
    phase: &str,
) -> Result<(), String> {
    let checks = report["checks"]
        .as_array()
        .ok_or("desktop scenario checks missing")?;
    for scenario in desktop_scenarios(phase)? {
        let matches: Vec<_> = checks
            .iter()
            .filter(|c| {
                c["id"] == DESKTOP_SCENARIO_CHECK && c["details"]["scenario"] == scenario.name
            })
            .collect();
        let [check] = matches.as_slice() else {
            return Err(format!(
                "{phase} desktop scenario {} needs exactly one recorded check",
                scenario.name
            ));
        };
        let details = &check["details"];
        let name = details["report"]["path"].as_str().unwrap_or_default();
        if check["outcome"] != "passed"
            || details["selected_phase"] != phase
            || details["function"] != scenario.function
            || details["checks"].as_array().is_none_or(Vec::is_empty)
            || !name.starts_with("reports/runs/")
        {
            return Err(format!(
                "{phase} desktop scenario {} has no passing bound evidence",
                scenario.name
            ));
        }
        let path = crate::paths::existing(root, name)?;
        if crate::evidence::file_hash(&path)? != details["report"]["sha256"]
            || crate::image::read_json(&path)?["success"] != true
        {
            return Err(format!(
                "{phase} desktop scenario {} report changed or did not succeed",
                scenario.name
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p12_runs_every_existing_desktop_scenario_and_earlier_phases_none() {
        assert!(desktop_scenarios("P11").unwrap().is_empty());
        assert!(desktop_scenarios("P18").is_err());
        let p12 = desktop_scenarios("P12").unwrap();
        assert_eq!(p12.len(), desktop_scenarios("P13").unwrap().len());
        assert_eq!(
            desktop_scenarios("P14").unwrap().len(),
            p12.len() + P14_DESKTOP_SCENARIOS.len()
        );
        assert_eq!(
            desktop_scenarios("P16").unwrap().len(),
            p12.len()
                + P14_DESKTOP_SCENARIOS.len()
                + P15_DESKTOP_SCENARIOS.len()
                + P16_DESKTOP_SCENARIOS.len()
        );
        assert_eq!(
            desktop_scenarios("P17").unwrap().len(),
            desktop_scenarios("P16").unwrap().len() + P17_DESKTOP_SCENARIOS.len()
        );
        let latest = desktop_scenarios("P17").unwrap();
        let mut names: Vec<_> = p12.iter().map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), p12.len(), "duplicate desktop scenario");
        // Every `desktop::*_test` entry point is either in the P11 desktop gate
        // or in the P12 list, so a new scenario cannot be silently left out.
        let source = include_str!("desktop.rs");
        let mut found = 0;
        for line in source.lines() {
            let Some(rest) = line.trim_start().strip_prefix("pub fn ") else {
                continue;
            };
            let name = rest.split('(').next().unwrap_or_default();
            if !name.ends_with("_test") {
                continue;
            }
            found += 1;
            assert!(
                DESKTOP_GATE_FUNCTIONS.contains(&name)
                    || P18_PENDING_FUNCTIONS.contains(&name)
                    || latest.iter().any(|s| s.function == name),
                "desktop scenario {name} is not in a desktop gate (scenarios::DESKTOP_SCENARIOS or a P14+ list)"
            );
        }
        assert!(found >= DESKTOP_GATE_FUNCTIONS.len() + 14);
        for name in DESKTOP_GATE_FUNCTIONS {
            assert!(source.contains(&format!("pub fn {name}(")), "{name}");
        }
    }

    #[test]
    fn timing_sensitive_guest_work_keeps_its_exclusive_slot() {
        // Cursor-latency limits, recorded drag/motion timings, guest
        // animation clocks, automatic host backend selection and races
        // between a crashed system process and its restart.
        let scenarios: Vec<_> = desktop_scenarios("P14")
            .unwrap()
            .into_iter()
            .filter(|s| s.exclusive)
            .map(|s| s.name)
            .collect();
        assert_eq!(
            scenarios,
            [
                "desktop-shell",
                "desktop-accelerated",
                "desktop-default",
                "desktop-software",
                "desktop-motion"
            ]
        );
        let steps: Vec<_> = crate::desktop::GATE_STEPS
            .iter()
            .filter(|s| s.exclusive)
            .map(|s| s.name)
            .collect();
        assert_eq!(
            steps,
            [
                "desktop-recovery",
                "desktop-wide",
                "desktop-resolutions",
                "desktop-native-default"
            ]
        );
        let suites: Vec<_> = ACCEPTANCE
            .iter()
            .filter(|s| exclusive_suite(s.name))
            .map(|s| s.name)
            .collect();
        assert_eq!(suites, ["motion-01", "recover-01", "vis-01"]);
        // Every scenario on the automatic (host-probing) backend is exclusive.
        let source = include_str!("desktop/resolution.rs");
        assert!(source.contains("super::native_default::verify(run, image)"));
        // Gate step, scenario and suite names are distinct (`--only`).
        let mut names: Vec<_> = crate::desktop::GATE_STEPS
            .iter()
            .map(|s| s.name)
            .chain(desktop_scenarios("P16").unwrap().iter().map(|s| s.name))
            .chain(ACCEPTANCE.iter().map(|s| s.name))
            .collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
        // Suites booting the shared Windows GUI stage are pre-staged once.
        for suite in ACCEPTANCE {
            let staged = matches!(suite.phase, Phase::P13)
                || matches!(suite.name, "desktop-a-shell" | "recover-01");
            assert_eq!(
                crate::desktop::uses_win32_stage(suite.name),
                staged,
                "{}",
                suite.name
            );
        }
    }

    #[test]
    fn every_acceptance_requirement_has_a_project_record() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../project/requirements.json"
        );
        let records: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let ids: Vec<_> = records["requirements"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["id"].as_str())
            .collect();
        for suite in ACCEPTANCE {
            for requirement in suite.requirements {
                assert!(
                    ids.contains(requirement),
                    "{} names unknown {requirement}",
                    suite.name
                );
            }
        }
    }

    #[test]
    fn desktop_scenario_evidence_must_be_complete_and_bound() {
        let workspace = crate::test_support::Workspace::new();
        let root = &workspace.root;
        let mut checks = Vec::new();
        for (index, scenario) in DESKTOP_SCENARIOS.iter().enumerate() {
            let name = format!("reports/runs/host-{index}-1/report.json");
            std::fs::create_dir_all(root.join(&name).parent().unwrap()).unwrap();
            std::fs::write(root.join(&name), br#"{"success":true}"#).unwrap();
            let digest = crate::evidence::file_hash(&root.join(&name)).unwrap();
            checks.push(
                json!({"id":DESKTOP_SCENARIO_CHECK,"outcome":"passed","details":{
                "scenario":scenario.name,"function":scenario.function,"selected_phase":"P12",
                "checks":["child"],"report":{"path":name,"sha256":digest}}}),
            );
        }
        let report = json!({"checks":checks});
        assert!(validate_desktop_scenarios(root, &report, "P12").is_ok());
        assert!(validate_desktop_scenarios(root, &json!({"checks":[]}), "P11").is_ok());
        // P13 needs its own selected-phase records.
        assert!(validate_desktop_scenarios(root, &report, "P13").is_err());
        let mut missing = report.clone();
        missing["checks"].as_array_mut().unwrap().remove(3);
        assert!(validate_desktop_scenarios(root, &missing, "P12").is_err());
        let mut duplicate = report.clone();
        let copy = duplicate["checks"][0].clone();
        duplicate["checks"].as_array_mut().unwrap().push(copy);
        assert!(validate_desktop_scenarios(root, &duplicate, "P12").is_err());
        for (field, value) in [
            ("function", json!("other_test")),
            ("selected_phase", json!("P11")),
            ("checks", json!([])),
            ("report", json!({"path":"../x/report.json","sha256":"0"})),
        ] {
            let mut bad = report.clone();
            bad["checks"][0]["details"][field] = value;
            assert!(
                validate_desktop_scenarios(root, &bad, "P12").is_err(),
                "{field}"
            );
        }
        let mut failed = report.clone();
        failed["checks"][0]["outcome"] = json!("failed");
        assert!(validate_desktop_scenarios(root, &failed, "P12").is_err());
        // A child report that failed, or changed after recording, cannot qualify.
        std::fs::write(
            root.join("reports/runs/host-0-1/report.json"),
            br#"{"success":false}"#,
        )
        .unwrap();
        assert!(validate_desktop_scenarios(root, &report, "P12").is_err());
    }

    #[test]
    fn p12_and_p13_reexecute_every_inherited_guest_scenario_cumulatively() {
        let foundation = cases("P11").unwrap();
        for phase in ["P12", "P13", "P14"] {
            let current = cases(phase).unwrap();
            assert_eq!(current.len(), foundation.len());
            for (old, new) in foundation.iter().zip(&current) {
                assert_eq!(
                    (old.id, old.mode, old.profile, &old.expected, &old.markers),
                    (new.id, new.mode, new.profile, &new.expected, &new.markers)
                );
            }
            let serial = foundation[0].markers.join("\n");
            assert_eq!(
                serial_matches(phase, &foundation[0].markers, &serial),
                serial_matches("P11", &foundation[0].markers, &serial)
            );
        }
        assert_eq!(
            check_id("P12").unwrap(),
            "p12-inherited-foundation-regressions"
        );
        assert_eq!(
            check_id("P13").unwrap(),
            "p13-inherited-foundation-regressions"
        );
        assert_eq!(
            check_id("P14").unwrap(),
            "p14-inherited-foundation-regressions"
        );
        for (phase, accepted) in [
            ("P12", true),
            ("P13", true),
            ("P14", true),
            ("P15", true),
            ("P16", true),
            ("P17", true),
            ("P18", false),
        ] {
            assert_eq!(
                crate::demo::arguments(
                    vec!["-display".into(), "none".into()],
                    false,
                    1234,
                    "11111111-1111-1111-1111-111111111111",
                    phase
                )
                .is_ok(),
                accepted
            );
        }
    }

    #[test]
    fn p13_acceptance_includes_every_p12_suite_and_p11_has_none() {
        assert!(acceptance_suites("P11").unwrap().is_empty());
        assert!(acceptance_suites("P18").is_err());
        let p12: Vec<_> = acceptance_suites("P12")
            .unwrap()
            .iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(
            p12,
            [
                "text-01",
                "a11y-01",
                "display-01",
                "motion-01",
                "component-gallery"
            ]
        );
        let p13: Vec<_> = acceptance_suites("P13")
            .unwrap()
            .iter()
            .map(|s| s.name)
            .collect();
        for name in &p12 {
            assert!(p13.contains(name), "{name}");
        }
        for name in [
            "win32-gui-f1",
            "win32-gui-f2",
            "win32-gui-f3",
            "scenario-141-5",
            "scenario-141-6",
        ] {
            assert!(p13.contains(&name) && !p12.contains(&name), "{name}");
        }
        let mut checks: Vec<_> = ACCEPTANCE.iter().map(|s| s.check).collect();
        checks.sort_unstable();
        checks.dedup();
        assert_eq!(checks.len(), ACCEPTANCE.len());
        for suite in ACCEPTANCE {
            assert!(!suite.spec.is_empty() && !suite.requirements.is_empty());
            assert!(
                suite
                    .spec
                    .iter()
                    .all(|line| line.starts_with("DOOR_OS_MASTER_SPECIFICATION.md:"))
            );
            assert_eq!(
                acceptance_suite(suite.family, suite.name).unwrap().check,
                suite.check
            );
        }
        assert!(acceptance_suite("ui", "win32-gui-f1").is_err());
        assert!(acceptance_suite("ui", "missing").is_err());
    }

    #[test]
    fn p14_acceptance_includes_every_p13_suite_plus_the_desktop_family() {
        let p13: Vec<_> = acceptance_suites("P13")
            .unwrap()
            .iter()
            .map(|s| s.name)
            .collect();
        let p14 = acceptance_suites("P14").unwrap();
        for name in &p13 {
            assert!(p14.iter().any(|s| s.name == *name), "{name}");
        }
        let added: Vec<_> = p14
            .iter()
            .filter(|s| s.phase == Phase::P14)
            .map(|s| (s.family, s.name, s.check))
            .collect();
        assert_eq!(
            added,
            [
                ("desktop", "desktop-a-shell", "p14-desktop-a-shell"),
                ("desktop", "recover-01", "p14-recover-01"),
                ("desktop", "desktop-a-files", "p14-desktop-a-files"),
                ("desktop", "desktop-a-apps", "p14-desktop-a-apps"),
                ("desktop", "sec-02-trusted", "p14-sec-02-trusted"),
                ("desktop", "vis-01", "p14-vis-01"),
            ]
        );
        assert_eq!(p14.len(), p13.len() + added.len());
        for (family, name, _) in &added {
            assert!(acceptance_suite(family, name).is_ok());
            assert!(acceptance_suite("ui", name).is_err());
        }
        // P13-selected evidence cannot satisfy the P14 gate.
        let checks: Vec<_> = acceptance_suites("P13")
            .unwrap()
            .iter()
            .map(|suite| {
                json!({"id":suite.check,"outcome":"passed",
                    "details":acceptance_record(suite,"P13",json!({"scenario":suite.name}))})
            })
            .collect();
        assert!(validate_acceptance(&json!({ "checks": checks }), "P14").is_err());
    }

    #[test]
    fn unimplemented_acceptance_suites_fail_clearly_and_never_pass() {
        // Suites without a registered runner make the gate say so; a phase
        // whose suites all have runners still needs passing evidence.
        for phase in ["P12", "P13", "P14"] {
            let suites = acceptance_suites(phase).unwrap();
            let registered = |name: &str| RUNNERS.iter().any(|(n, _)| *n == name);
            let empty = json!({"checks":[]});
            let missing_evidence = validate_acceptance(&empty, phase).unwrap_err();
            if suites.iter().all(|suite| registered(suite.name)) {
                assert!(acceptance_preflight(phase).is_ok());
                assert!(
                    missing_evidence.contains("no passing evidence"),
                    "{missing_evidence}"
                );
                continue;
            }
            let error = acceptance_preflight(phase).unwrap_err();
            assert!(
                error.contains("acceptance suite not implemented"),
                "{error}"
            );
            for suite in suites {
                if RUNNERS.iter().any(|(name, _)| *name == suite.name) {
                    continue;
                }
                assert!(error.contains(suite.name), "{error}");
                assert!(
                    runner(suite)
                        .err()
                        .unwrap()
                        .contains("acceptance suite not implemented")
                );
            }
            assert!(missing_evidence.contains("no passing evidence"));
        }
        assert!(acceptance_preflight("P11").is_ok());
        assert!(validate_acceptance(&json!({"checks":[]}), "P11").is_ok());
    }

    #[test]
    fn acceptance_evidence_is_bound_to_suite_identity_and_selected_phase() {
        let p12 = acceptance_suites("P12").unwrap();
        let checks: Vec<_> = p12
            .iter()
            .map(|suite| {
                json!({"id":suite.check,"outcome":"passed",
                    "details":acceptance_record(suite,"P12",json!({"scenario":suite.name}))})
            })
            .collect();
        let report = json!({ "checks": checks });
        assert!(validate_acceptance(&report, "P12").is_ok());
        // P13 also requires its own five suites and P13-selected records.
        assert!(validate_acceptance(&report, "P13").is_err());
        for (field, value) in [("outcome", json!("failed")), ("id", json!("p12-other"))] {
            for index in 0..p12.len() {
                let mut bad = report.clone();
                bad["checks"][index][field] = value.clone();
                assert!(validate_acceptance(&bad, "P12").is_err(), "{field}");
            }
        }
        for (field, value) in [
            ("suite", json!("a11y-01")),
            ("requirement", json!("A11Y-01")),
            ("selected_phase", json!("P11")),
            ("result", json!({})),
            ("result", Value::Null),
        ] {
            let mut bad = report.clone();
            bad["checks"][0]["details"][field] = value;
            assert!(validate_acceptance(&bad, "P12").is_err(), "{field}");
        }
    }

    #[test]
    fn p11_preserves_all_prior_guest_scenarios_before_separate_desktop_gates() {
        let previous = cases("P10").unwrap();
        let current = cases("P11").unwrap();
        assert_eq!(current.len(), 42);
        assert_eq!(current.len(), previous.len());
        for (old, new) in previous.iter().zip(current) {
            assert_eq!(
                (old.id, old.mode, old.profile, &old.expected, &old.markers),
                (new.id, new.mode, new.profile, &new.expected, &new.markers)
            );
        }
        assert_eq!(check_id("P11").unwrap(), "p11-native-window-regressions");
        assert!(
            crate::demo::arguments(
                vec!["-display".into(), "none".into()],
                false,
                1234,
                "11111111-1111-1111-1111-111111111111",
                "P11"
            )
            .is_ok()
        );
    }

    #[test]
    fn p05_retains_regressions_and_requires_attributed_native_execution() {
        let old = cases("P04").unwrap();
        let cases = cases("P05").unwrap();
        assert_eq!(cases.len(), 15);
        for (prior, new) in old.iter().zip(&cases) {
            assert_eq!(
                (prior.id, prior.mode, prior.profile, &prior.expected),
                (new.id, new.mode, new.profile, &new.expected)
            );
            assert!(
                prior
                    .markers
                    .iter()
                    .all(|marker| new.markers.contains(marker))
            );
        }
        let case = &cases[0];
        let mut lines = case
            .markers
            .iter()
            .filter(|marker| !marker.starts_with("DOOR_P05 "))
            .map(|marker| {
                if marker.starts_with("DOOR_P04 TLB") {
                    format!("{marker} rounds=32")
                } else {
                    (*marker).to_owned()
                }
            })
            .collect::<Vec<_>>();
        lines.extend(P05_SUMMARIES.iter().map(|marker| (*marker).to_owned()));
        lines.push(
            "DOOR_P05 MEMORY PASS allocated_pages=100 kernel_table_pages=24 copy_faults=6".into(),
        );
        lines.push("DOOR_P05 STARTUP_ROLLBACK PASS stages=16 leaks=0".into());
        lines.extend(
            P05_PROCESSES
                .iter()
                .enumerate()
                .map(|(index, marker)| format!("{marker} pid={}", index + 101)),
        );
        lines.extend(
            P05_USER_RECORDS
                .iter()
                .map(|(marker, role)| format!("DOOR_USER pid={} {marker}", role + 101)),
        );
        let serial = lines.join("\n");
        assert!(serial_matches("P05", &case.markers, &serial));
        for marker in p05_markers() {
            assert!(!serial_matches(
                "P05",
                &case.markers,
                &serial.replace(marker, "")
            ));
        }
        for bad in [
            serial.replace("ring3=1", "ring3=0"),
            serial.replace("allocated_pages=100", "allocated_pages=0"),
            serial.replace("kernel_table_pages=24", "kernel_table_pages=65"),
            serial.replace("copy_faults=6", "copy_faults=5"),
            serial.replace("copy_faults=6", "copy_faults=6 copy_faults=6"),
            serial.replace("stages=16", "stages=11"),
            serial.replace("stages=16", "stages=27"),
            serial.replace("stages=16", "stages=17"),
            serial.replace("stages=16 leaks=0", "stages=16 leaks=1"),
            format!(
                "{serial}\nDOOR_P05 MEMORY PASS allocated_pages=1 kernel_table_pages=1 copy_faults=6"
            ),
            serial.replace("cancellation=1 deadline=1", "cancellation=0 deadline=1"),
            serial.replace(
                "pid=101 DOOR_P05 IPC_LIFETIME",
                "pid=102 DOOR_P05 IPC_LIFETIME",
            ),
            serial.replace("retired=6 leaked=0", "retired=6 leaked=0 leaked=1"),
            serial.replace("role=3 exit=-14", "role=3 exit=0"),
            serial.replace("role=6 exit=-257", "role=6 exit=-14"),
            serial.replace("pid=102", "pid=101"),
            serial.replace("pid=101 DOOR_P05 CLIENT", "pid=102 DOOR_P05 CLIENT"),
            serial.replace("DOOR_USER pid=105 DOOR_P05 RAW_ABI", "DOOR_P05 RAW_ABI"),
            serial.replace("pid=104", "pid=0104"),
            format!("{serial}\nDOOR_P05 PROCESS role=1 exit=11 pid=900"),
            format!("{serial}\nDOOR_USER pid=105 DOOR_P05 RAW_ABI PASS"),
            format!("{serial}\nDOOR_P05 CLEANUP FAIL"),
        ] {
            assert!(
                !serial_matches("P05", &case.markers, &bad),
                "accepted {bad}"
            );
        }
        assert_eq!(check_id("P05").unwrap(), "p05-protected-native");
    }

    #[test]
    fn p04_retains_all_prior_scenarios_and_binds_distinct_topologies() {
        let old = cases("P02").unwrap();
        let middle = cases("P03").unwrap();
        let current = cases("P04").unwrap();
        assert_eq!((old.len(), middle.len(), current.len()), (6, 12, 15));
        for prior in middle {
            let matching = current
                .iter()
                .filter(|c| c.id == prior.id)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1);
            let new = matching[0];
            assert_eq!(
                (new.mode, new.profile, &new.expected),
                (prior.mode, prior.profile, &prior.expected)
            );
            for marker in prior.markers {
                assert!(new.markers.contains(&marker));
            }
        }
        for (id, mode, profile, cpus) in [
            ("normal-smp2", "normal", SMP2_PROFILE, 2),
            ("normal-smp4", "normal", SMP4_PROFILE, 4),
            ("smp-degraded", "smp-degraded", SMP2_PROFILE, 1),
        ] {
            let case = current.iter().find(|case| case.id == id).unwrap();
            assert_eq!(
                (case.mode, case.profile, &case.expected),
                (mode, profile, &Outcome::Passed)
            );
            assert!(
                case.markers
                    .contains(&format!("DOOR_P04 TLB PASS cpus={cpus}").as_str())
            );
        }
        for phase in ["P01", "P18", "P99", ""] {
            assert!(cases(phase).is_err());
            assert!(check_id(phase).is_err());
        }
    }

    #[test]
    fn p04_rejects_missing_or_conflicting_online_and_retirement_evidence() {
        let cases = cases("P04").unwrap();
        let case = cases.iter().find(|case| case.id == "normal-smp2").unwrap();
        let serial = case
            .markers
            .iter()
            .map(|marker| {
                if marker.starts_with("DOOR_P04 TLB ") {
                    format!("{marker} rounds=200")
                } else {
                    (*marker).to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(serial_matches("P04", &case.markers, &serial));
        for invalid in [
            serial.replace("online=2", "online=1"),
            serial.replace("discovered=2", "discovered=4"),
            serial.replace("degraded=0", "degraded=01"),
            serial.replace("cpus=2", "cpus=20"),
            serial.replace("rounds=200", "rounds=0"),
            serial.replace(" rounds=200", ""),
            serial.replace("rounds=200", "rounds=200 rounds=10"),
            serial.replace("degraded=0", "degraded=0 online=1"),
            serial.replace("rounds=200", "rounds=200 cpus=4"),
            format!("{serial}\nDOOR_P04 SMP PASS discovered=2 online=1 degraded=1"),
            format!("{serial}\nDOOR_P04 TASKS PASS"),
        ] {
            assert!(
                !serial_matches("P04", &case.markers, &invalid),
                "accepted {invalid}"
            );
        }
        for marker in &case.markers {
            assert!(!serial_matches(
                "P04",
                &case.markers,
                &serial.replace(marker, "")
            ));
        }
        let degraded = cases.iter().find(|case| case.id == "smp-degraded").unwrap();
        assert!(!serial_matches("P04", &degraded.markers, &serial));
        // Historical marker semantics remain unchanged, including embedded diagnostic text.
        assert!(serial_matches(
            "P02",
            &["intentional P02 panic"],
            "PANIC: intentional P02 panic at location"
        ));
    }
}
