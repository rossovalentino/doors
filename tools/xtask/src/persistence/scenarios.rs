//! P07 scenarios retain the P06 suite and add real journal interruption/recovery.
use crate::{emulator::Outcome, scenarios::Scenario};
pub const SCHEDULE: &[(&str, &str)] = &[
    ("persist-save", "persist-save"),
    ("persist-reopen", "persist-reopen"),
    ("journal-prepare", "journal-prepare"),
    ("journal-prepare-recovery", "recover-old"),
    ("journal-prepare-reopen", "persist-old"),
    ("journal-commit", "journal-commit"),
    ("journal-commit-recovery", "recover-new"),
    ("journal-commit-reopen", "persist-new"),
    ("journal-first-home", "journal-first-home"),
    ("journal-first-home-recovery", "recover-new"),
    ("journal-first-home-reopen", "persist-new"),
    ("journal-home", "journal-home"),
    ("journal-home-recovery", "recover-new"),
    ("journal-home-reopen", "persist-new"),
    ("journal-checkpoint", "journal-checkpoint"),
    ("journal-checkpoint-recovery", "recover-new"),
    ("journal-checkpoint-reopen", "persist-new"),
    ("persist-full", "persist-full"),
];
pub fn cases(base: &[&'static str]) -> Vec<Scenario> {
    SCHEDULE
        .iter()
        .map(|&(id, mode)| {
            let crash = super::interruption_stage(mode);
            let mut markers: Vec<_> = base
                .iter()
                .copied()
                .filter(|m| crash.is_none() || *m != "KERNEL_DIAGNOSTIC PASS")
                .collect();
            if let Some(stage) = crash {
                markers.push(match stage {
                    "prepare" => "DOOR_P07 INTERRUPT stage=prepare",
                    "commit" => "DOOR_P07 INTERRUPT stage=commit",
                    "first_home" => "DOOR_P07 INTERRUPT stage=first_home",
                    "home" => "DOOR_P07 INTERRUPT stage=home",
                    _ => "DOOR_P07 INTERRUPT stage=checkpoint",
                });
            } else {
                markers.extend([
                    "DOOR_P07 STACK PASS",
                    "DOOR_P07 CLEANUP PASS dma_quiesced=1 pages_reclaimed=8",
                ]);
                if mode == "persist-save" {
                    markers.push("DOOR_P07 LIFETIME PASS replace=1 delete=1 truncate=1 stale=1");
                }
                if mode == "persist-full" {
                    markers.push("DOOR_P07 CAPACITY PASS");
                } else {
                    let new = matches!(mode, "recover-new" | "persist-new");
                    let replay = mode == "recover-new" && id != "journal-checkpoint-recovery";
                    markers.extend([
                        if new {
                            "DOOR_P07 PERSIST PASS state=new bytes=6000 crc32=69f67483"
                        } else {
                            "DOOR_P07 PERSIST PASS state=old bytes=6000 crc32=01ea357b"
                        },
                        "DOOR_P07 SECURITY PASS acl=1 sharing=1 rename=1 metadata=1",
                        "DOOR_P07 CONFIG PASS native=1 registry=1 typed=1 unrelated=1",
                    ]);
                    markers.push(match (new, replay) {
                        (false, _) => "DOOR_P07 RECOVERY PASS state=old replayed=0",
                        (true, false) => "DOOR_P07 RECOVERY PASS state=new replayed=0",
                        (true, true) => "DOOR_P07 RECOVERY PASS state=new replayed=1",
                    });
                }
            }
            Scenario {
                id,
                mode,
                profile: super::PROFILE,
                expected: if crash.is_some() {
                    Outcome::Interrupted
                } else {
                    Outcome::Passed
                },
                markers,
            }
        })
        .collect()
}
fn decimal(text: &str) -> Option<u64> {
    let n = text.parse::<u64>().ok()?;
    (n.to_string() == text).then_some(n)
}
pub fn serial_matches(markers: &[&str], serial: &str) -> bool {
    markers
        .iter()
        .filter(|m| m.starts_with("DOOR_P07 "))
        .all(|marker| {
            let name = marker.split_whitespace().nth(1).unwrap_or("");
            let prefix = format!("DOOR_P07 {name} ");
            let lines: Vec<_> = serial
                .lines()
                .filter(|line| line.starts_with(&prefix))
                .collect();
            if lines.len() != 1 {
                return false;
            }
            let line = lines[0];
            match name {
                "RECOVERY" => line
                    .strip_prefix(&format!("{marker} generation="))
                    .and_then(decimal)
                    .is_some(),
                "INTERRUPT" => line
                    .strip_prefix(&format!("{marker} generation="))
                    .and_then(decimal)
                    .is_some_and(|n| n > 0),
                "STACK" => line
                    .strip_prefix("DOOR_P07 STACK PASS sampled_remaining=")
                    .and_then(|s| s.strip_suffix(" stack_bytes=131072"))
                    .and_then(decimal)
                    .is_some_and(|n| (16384..=131072).contains(&n)),
                "CAPACITY" => line
                    .strip_prefix("DOOR_P07 CAPACITY PASS files=")
                    .and_then(|s| s.strip_suffix(" free_blocks=0 rollback=1 preserved=1"))
                    .and_then(decimal)
                    .is_some_and(|n| n == 9),
                _ => line == *marker,
            }
        })
}
