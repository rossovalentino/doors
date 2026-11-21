//! Actual Monitor telemetry and interaction on isolated Storage.
use super::*;
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("monitor-private.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, "1920x1080")?;
    let mut visits = Vec::new();
    let result = (|| -> Result<Value, String> {
        let mut c = connection(&record)?;
        let mut pointer = (960, 540);
        thread::sleep(Duration::from_secs(2));
        resolution::activate(&run.root, &record, &mut c, 5)?;
        key(&mut c, &["meta_l", "up"])?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_MONITOR SETTLED",
            Duration::from_secs(30),
        )?;
        let pause_at = serial(&run.root, &record)?.len();
        // Maximized 1920x1032 client (desktop/system-monitor/src/presentation.rs):
        // header commands end at width - 16 = 1904 on the row y 58..90, the
        // last being "View"; its 240 px menu is right-aligned to it (x 1664..1904)
        // from y 94, entries from y 98: Refresh 36, separator 9, heading 28,
        // High/Normal/Low/Paused 36 each, so Paused is y 279..315.
        click(&mut c, &mut pointer, 1890, 74)?;
        click(&mut c, &mut pointer, 1784, 297)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !serial(&run.root, &record)?
            .get(pause_at..)
            .unwrap_or("")
            .contains("DOOR_MONITOR PAUSED value=1")
        {
            if Instant::now() >= deadline {
                return Err("fresh Monitor pause absent".into());
            }
            thread::sleep(Duration::from_millis(50));
        }
        scene::capture_settled(run, &mut c, "monitor-performance-paused", &[])?;
        key(&mut c, &["ctrl", "f"])?;
        type_line(&mut c, "Task")?;
        scene::capture_settled(run, &mut c, "monitor-filtered", &[])?;
        // Task Manager destinations: Processes..Services (0-6) then pinned
        // Settings (7); Performance (1) again, then its CPU/Memory/Disk tiles
        // (routes 20-22). Navigation rows: x 4..216, y = 96 + 40 * i, 36 tall;
        // Settings is pinned at height - 44. Tiles: x 228..472, y = 100 + 76 * i.
        for (index, page) in (0..8).chain([1, 20, 21, 22]).enumerate() {
            let before = serial(&run.root, &record)?.len();
            if page >= 20 {
                click(&mut c, &mut pointer, 350, 134 + (page - 20) * 76)?;
            } else {
                let y = if page == 7 {
                    1032 - 26
                } else {
                    114 + page * 40
                };
                click(&mut c, &mut pointer, 80, y)?;
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            let marker = format!("DOOR_MONITOR PAGE id={page}");
            loop {
                if serial(&run.root, &record)?
                    .get(before..)
                    .unwrap_or("")
                    .lines()
                    .any(|l| l.ends_with(&marker))
                {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(format!("fresh Monitor page {page} absent"));
                }
                thread::sleep(Duration::from_millis(50));
            }
            scene::capture_settled(run, &mut c, &format!("monitor-page-{index}-{page}"), &[])?;
            visits.push((page as u32, before));
        }

        thread::sleep(Duration::from_secs(2));
        key(&mut c, &["alt", "f4"])?;
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(25),
        )?;
        Ok(json!(pause_at))
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = scene::capture_settled(run, &mut c, "monitor-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    let pause_at = result?.as_u64().ok_or("Monitor pause offset")? as usize;
    seal_serial(&run.root, &mut record)?;
    let sealed = serial(&run.root, &record)?;
    let retired = shutdown::retirement_commands(&sealed, &[11])?;
    let counters = canonical(&sealed, &retired, pause_at)?;
    let routes =
        reference_routes::validate(&sealed, &retired, 11, 14, "DOOR_MONITOR PAGE", &visits)?;
    run.check("monitor-all-page-navigation", routes);
    run.check("monitor-native-telemetry",json!({"session":record,"retirement":retired,"observed":counters,"scope":"Real sanitized kernel timer observations/private committed memory/device request counts; no full Task Manager/Win11 fidelity/CPU-time/GPU/network qualification","owner_volume_used":false}));
    Ok(())
}

fn canonical(text: &str, retirement: &Value, pause_at: usize) -> Result<Value, String> {
    use std::collections::{BTreeMap, BTreeSet};
    let pid = retirement["applications"]["11"]
        .as_u64()
        .ok_or("Monitor launched PID")?;
    let identities = retirement["identities"]
        .as_array()
        .ok_or("Monitor identities")?;
    let identity = identities
        .iter()
        .find(|p| p["fields"]["pid"] == pid)
        .ok_or("Monitor retired identity")?;
    if identity["fields"]["manifest"] != 14 {
        return Err("Monitor manifest differs".into());
    }
    let end = identity["line"].as_u64().ok_or("Monitor retirement line")? as usize;
    let launch_suffix = format!(" DOOR_P12 APP_LAUNCHED app=11 pid={pid}");
    let launch = text
        .lines()
        .position(|l| l.ends_with(&launch_suffix))
        .ok_or("Monitor launch")?;
    let mut sample = None;
    let mut settled = None;
    let mut paused = None;
    let mut offset = 0;
    for (i, raw) in text.split_inclusive('\n').enumerate() {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.contains("DOOR_MONITOR SAMPLE")
            || line.contains("DOOR_MONITOR SETTLED")
            || line.contains("DOOR_MONITOR PAUSED")
        {
            if i <= launch || i >= end {
                return Err("Monitor marker outside process lifetime".into());
            }
            let prefix = format!("DOOR_USER pid={pid} DOOR_MONITOR ");
            let payload = line.strip_prefix(&prefix).ok_or("foreign Monitor marker")?;
            if let Some((kind, fields)) = payload
                .split_once(' ')
                .filter(|(kind, _)| matches!(*kind, "SAMPLE" | "SETTLED"))
            {
                let mut values = BTreeMap::new();
                for token in fields.split_whitespace() {
                    let (key, value) = token.split_once('=').ok_or("Monitor sample field")?;
                    let n = value.parse::<u64>().map_err(|_| "Monitor sample number")?;
                    if n.to_string() != value || values.insert(key, n).is_some() {
                        return Err("noncanonical Monitor sample field".into());
                    }
                }
                if values.keys().copied().collect::<BTreeSet<_>>()
                    != BTreeSet::from(["processes", "managed", "allocated", "observations", "busy"])
                    || values["processes"]
                        != 6 + (super::shutdown::search_hosts(text).len()
                            + super::shutdown::ime_hosts(text).len()
                            + super::shutdown::network_hosts(text).len())
                            as u64
                    || values["managed"] <= values["allocated"]
                    || values["allocated"] == 0
                    || values["observations"] < values["busy"]
                {
                    return Err("Monitor real counters invalid/duplicate".into());
                }
                let slot = if kind == "SAMPLE" {
                    &mut sample
                } else {
                    &mut settled
                };
                if slot.is_some() {
                    return Err("Duplicate Monitor sample".into());
                }
                *slot = Some((i, json!(values)));
            } else if payload == "PAUSED value=1" && offset >= pause_at && paused.is_none() {
                paused = Some(i);
            } else {
                return Err("noncanonical or stale Monitor pause".into());
            }
        }
        offset += raw.len();
    }
    let (sample_line, values) = sample.ok_or("Monitor sample absent")?;
    let (settled_line, settled_values) = settled.ok_or("Monitor settled sample absent")?;
    let pause_line = paused.ok_or("Monitor pause absent")?;
    if sample_line >= settled_line || settled_line >= pause_line {
        return Err("Monitor pause precedes sample".into());
    }
    let difference = |key: &str| {
        settled_values[key]
            .as_u64()
            .and_then(|end| end.checked_sub(values[key].as_u64()?))
            .ok_or("Monitor counter reset")
    };
    let total = difference("observations")?;
    let busy = difference("busy")?;
    if total == 0 || busy > total {
        return Err("Invalid Monitor settled interval".into());
    }
    Ok(
        json!({"identity":identity["fields"],"sample_line":sample_line,"settled_line":settled_line,"pause_line":pause_line,"counters":values,"settled_counters":settled_values,"settled_interval":{"observations":total,"busy":busy},"timing_scope":"coarse timer observations, not CPU-time or responsiveness proof"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn samples_require_canonical_fields_authenticated_actor_and_order() {
        let text = "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED app=11 pid=6\nDOOR_USER pid=6 DOOR_MONITOR SAMPLE processes=6 managed=100 allocated=50 observations=20 busy=10\nDOOR_USER pid=6 DOOR_MONITOR SETTLED processes=6 managed=100 allocated=50 observations=420 busy=40\nDOOR_USER pid=6 DOOR_MONITOR PAUSED value=1\nDOOR_P08 PROCESS manifest=14 pid=6 instance=8 uid=1001 session=1 epoch=1 exit=0\n";
        let retired = json!({"applications":{"11":6},"identities":[{"line":4,"fields":{"manifest":14,"pid":6,"instance":8}}]});
        assert!(canonical(text, &retired, 0).is_ok());
        let crlf = text.replace('\n', "\r\n");
        let pause_at = crlf.find("DOOR_USER pid=6 DOOR_MONITOR PAUSED").unwrap();
        assert!(canonical(&crlf, &retired, pause_at).is_ok());
        assert!(canonical(&crlf, &retired, pause_at + 1).is_err());
        for bad in [
            text.replace("managed=100", "wrong=100"),
            text.replace("managed=100", "managed=0100"),
            text.replace(
                "DOOR_USER pid=6 DOOR_MONITOR",
                "DOOR_USER pid=7 DOOR_MONITOR",
            ),
            text.replace("busy=10", "busy=10 busy=10"),
            text.replace("PAUSED value=1", "PAUSED value=01"),
            text.replace("observations=420", "observations=19"),
            text.replace("busy=40", "busy=411"),
        ] {
            assert!(canonical(&bad, &retired, 0).is_err());
        }
        assert!(canonical(text, &retired, text.len()).is_err());
    }
}
