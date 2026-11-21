//! Real windows and owned tab handoff, on a private disk and fresh kernel image.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn fresh(run: &Run, record: &Value, before: usize, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if serial(&run.root, record)?
            .get(before..)
            .unwrap_or("")
            .lines()
            .any(|line| line.starts_with("DOOR_USER pid=") && line.contains(marker))
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("fresh multiwindow observation absent: {marker}"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
/// One parsed marker: kernel pid and its numeric fields.
type Record = (u64, BTreeMap<String, u64>);
/// A parsed marker with its serial byte offset and line index.
type Observation = (u64, BTreeMap<String, u64>, usize, usize);
fn records(text: &str, marker: &str) -> Result<Vec<Record>, String> {
    let mut result = Vec::new();
    for line in text.lines() {
        let Some((prefix, body)) = line.split_once(&format!(" {marker} ")) else {
            continue;
        };
        let pid = prefix
            .strip_prefix("DOOR_USER pid=")
            .ok_or("multiwindow actor prefix")?;
        let pid_value: u64 = pid.parse().map_err(|_| "multiwindow actor")?;
        if pid_value == 0 || pid_value.to_string() != pid {
            return Err("multiwindow canonical actor".into());
        }
        let mut fields = BTreeMap::new();
        for token in body.split_whitespace() {
            // Window titles are descriptive, not proof or authority.
            if token.starts_with("title=") {
                break;
            }
            let (k, v) = token.split_once('=').ok_or("multiwindow field")?;
            let n: u64 = v.parse().map_err(|_| "multiwindow field number")?;
            if n == 0 || n.to_string() != v || fields.insert(k.into(), n).is_some() {
                return Err("multiwindow canonical field".into());
            }
        }
        result.push((pid_value, fields));
    }
    Ok(result)
}
fn observations(text: &str, marker: &str) -> Result<Vec<Observation>, String> {
    let mut result = Vec::new();
    let mut offset = 0;
    for (line, raw) in text.split_inclusive('\n').enumerate() {
        for (pid, fields) in records(raw.trim_end(), marker)? {
            result.push((pid, fields, offset, line));
        }
        offset += raw.len();
    }
    Ok(result)
}
const GESTURES: [(i64, &str); 4] = [
    (1, "TAB_DETACHED"),
    (1, "TAB_MOVED"),
    (10, "TAB_DETACHED"),
    (10, "TAB_MOVED"),
];
fn verify_actors(
    text: &str,
    retired: &Value,
    gestures: &[(i64, &str, usize, usize)],
) -> Result<Value, String> {
    if gestures.len() != 4
        || gestures
            .iter()
            .zip(GESTURES)
            .any(|((a, k, _, _), (b, l))| *a != b || *k != l)
    {
        return Err("both apps require ordered detach and attach gestures".into());
    }
    let created = observations(text, "DOOR_P12 WINDOW_CREATED")?;
    let committed = observations(text, "DOOR_P12 WINDOW_COMMITTED")?;
    let identities = retired["identities"]
        .as_array()
        .ok_or("retired identities")?;
    let init = identities
        .iter()
        .find(|v| v["fields"]["manifest"] == 1)
        .and_then(|v| v["fields"]["pid"].as_u64())
        .ok_or("retired Init")?;
    let mut owners = BTreeMap::new();
    let mut actors = BTreeMap::new();
    let mut summary = Vec::new();
    for command in [1, 2, 3, 9, 10, 11] {
        let pid = retired["applications"][command.to_string()]
            .as_u64()
            .ok_or("retired app")?;
        let matching: Vec<_> = identities
            .iter()
            .filter(|v| v["fields"]["pid"] == pid)
            .collect();
        if matching.len() != 1 {
            return Err("one retired app incarnation required".into());
        }
        let identity = matching[0];
        let expected = match command {
            10 => 13,
            11 => 14,
            _ => 12,
        };
        if identity["fields"]["manifest"] != expected
            || identity["fields"]["instance"].as_u64().unwrap_or(0) == 0
            || identity["fields"]["uid"] != 1001
            || identity["fields"]["session"].as_u64().unwrap_or(0) == 0
            || identity["fields"]["epoch"].as_u64().unwrap_or(0) == 0
            || identity["fields"]["exit"] != 0
        {
            return Err("multiwindow app authority differs".into());
        }
        let end = identity["line"].as_u64().ok_or("retirement line")? as usize;
        let launch = format!("DOOR_USER pid={init} DOOR_P12 APP_LAUNCHED app={command} pid={pid}");
        let launches: Vec<_> = text
            .lines()
            .enumerate()
            .filter(|(_, l)| *l == launch)
            .map(|(i, _)| i)
            .collect();
        if launches.len() != 1 || launches[0] >= end {
            return Err("unique native launch lifetime required".into());
        }
        actors.insert(pid, (launches[0], end));
        let entries: Vec<_> = created.iter().filter(|(p, _, _, _)| *p == pid).collect();
        if entries.len() < 2 {
            return Err(format!("app{command} lacks two actual windows"));
        }
        let mut ids = Vec::new();
        for (_, fields, byte, line) in entries {
            if fields.len() != 1 || *line <= launches[0] || *line >= end {
                return Err("window creation outside actor lifetime".into());
            }
            let id = *fields.get("id").ok_or("window ID")?;
            let first: Vec<_> = committed
                .iter()
                .filter(|(p, f, _, _)| *p == pid && f.get("id") == Some(&id))
                .collect();
            if first.len() != 1
                || first[0].1.len() != 1
                || first[0].3 <= *line
                || first[0].3 >= end
                || first[0].2 <= *byte
                || owners.insert(id, (pid, first[0].2)).is_some()
            {
                return Err("owned window commit/order missing or replayed".into());
            }
            ids.push(id);
        }
        summary.push(json!({"app":command,"identity":identity["fields"],"windows":ids}));
    }
    if created.iter().any(|(p, _, _, _)| !actors.contains_key(p))
        || committed.iter().any(|(p, f, _, _)| {
            f.get("id")
                .is_none_or(|id| owners.get(id).is_none_or(|v| v.0 != *p))
        })
    {
        return Err("foreign window observation".into());
    }
    let mut pairs = Vec::new();
    for (command, kind, before, after) in gestures {
        let pid = retired["applications"][command.to_string()]
            .as_u64()
            .ok_or("tab actor")?;
        let bytes = text
            .get(*before..*after)
            .ok_or("fresh gesture byte range")?;
        let moves = observations(bytes, &format!("DOOR_P12 {kind}"))?;
        let other = if *kind == "TAB_DETACHED" {
            "TAB_MOVED"
        } else {
            "TAB_DETACHED"
        };
        if moves.len() != 1 || !records(bytes, &format!("DOOR_P12 {other}"))?.is_empty() {
            return Err("wrong/duplicate/missing gesture kind".into());
        }
        let (actor, fields, at, _) = &moves[0];
        let required = if *kind == "TAB_DETACHED" {
            BTreeSet::from(["from", "to", "tab"])
        } else {
            BTreeSet::from(["from", "to"])
        };
        if fields.keys().map(String::as_str).collect::<BTreeSet<_>>() != required {
            return Err("tab gesture field set".into());
        }
        let (from, to) = (fields["from"], fields["to"]);
        let transfer = *before + *at;
        if *actor != pid
            || from == to
            || [from, to].iter().any(|id| {
                owners
                    .get(id)
                    .is_none_or(|(p, committed)| *p != pid || *committed >= transfer)
            })
        {
            return Err("tab actor/committed ownership mismatch".into());
        }
        let line = text[..transfer].lines().count();
        let (launch, end) = actors[&pid];
        if line <= launch || line >= end {
            return Err("tab gesture outside native incarnation".into());
        }
        pairs.push((from, to));
    }
    if pairs[0] != (pairs[1].1, pairs[1].0) || pairs[2] != (pairs[3].1, pairs[3].0) {
        return Err("reattach did not reverse its detached document route".into());
    }
    Ok(
        json!({"apps":summary,"gestures":gestures.iter().map(|(a,k,b,e)|json!({"app":a,"kind":k,"before_byte":b,"after_byte":e})).collect::<Vec<_>>(),"admission":"successful capture/commit; physical scanout is asserted separately by screenshots"}),
    )
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("multiwindow-private.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, "1920x1080")?;
    let mut gestures = Vec::new();
    let result = (|| -> Result<(), String> {
        let mut c = connection(&record)?;
        let mut p = (960, 540);
        resolution::activate(&run.root, &record, &mut c, 1)?;
        type_line(&mut c, "P3 2 2 255 255 0 0 0 255 0 0 0 255 255 255 255")?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["ctrl", "s"])?;
        fresh(run, &record, before, "DOOR_P11 NOTES SAVED")?;
        resolution::activate(&run.root, &record, &mut c, 0)?;
        key(&mut c, &["ctrl", "l"])?;
        type_line(&mut c, r"C:\Users\1001\notes")?;
        key(&mut c, &["home"])?;
        key(&mut c, &["f2"])?;
        key(&mut c, &["ctrl", "a"])?;
        let before = serial(&run.root, &record)?.len();
        type_line(&mut c, "multi-colors.ppm")?;
        fresh(run, &record, before, "FILES RENAME COMMITTED")?;
        key(&mut c, &["meta_l", "up"])?;
        key(&mut c, &["ctrl", "t"])?;
        let before = serial(&run.root, &record)?.len();
        scene::drag(&mut c, &mut p, (400, 20), (1000, 400))?;
        fresh(run, &record, before, "DOOR_P12 TAB_DETACHED")?;
        gestures.push((1, "TAB_DETACHED", before, serial(&run.root, &record)?.len()));
        scene::capture_settled(run, &mut c, "explorer-detached-native", &[])?;
        let before = serial(&run.root, &record)?.len();
        scene::drag(&mut c, &mut p, (1000, 400), (700, 20))?;
        fresh(run, &record, before, "DOOR_P12 TAB_MOVED")?;
        gestures.push((1, "TAB_MOVED", before, serial(&run.root, &record)?.len()));
        scene::capture_settled(run, &mut c, "explorer-reattached-native", &[])?;

        resolution::activate(&run.root, &record, &mut c, 4)?;
        key(&mut c, &["meta_l", "up"])?;
        for _ in 0..2 {
            let before = serial(&run.root, &record)?.len();
            key(&mut c, &["ctrl", "o"])?;
            type_line(&mut c, r"C:\Users\1001\notes\multi-colors.ppm")?;
            fresh(run, &record, before, "image=1")?;
        }
        let before = serial(&run.root, &record)?.len();
        // Preview's caption tabs share Explorer's strip (x=8, ~294 px tabs):
        // the second tab spans ~302..596, so grab inside it.
        // Same drop point as Explorer: the 960x600 torn-off window fits
        // on screen there, so `initial_window` keeps the tab under the pointer.
        scene::drag(&mut c, &mut p, (450, 20), (1000, 400))?;
        fresh(run, &record, before, "DOOR_P12 TAB_DETACHED")?;
        gestures.push((
            10,
            "TAB_DETACHED",
            before,
            serial(&run.root, &record)?.len(),
        ));
        scene::capture_settled(run, &mut c, "preview-detached-native", &[])?;
        let before = serial(&run.root, &record)?.len();
        // The detached window keeps the dragged tab under the drop point,
        // as for Explorer above.
        scene::drag(&mut c, &mut p, (1000, 400), (900, 20))?;
        fresh(run, &record, before, "DOOR_P12 TAB_MOVED")?;
        gestures.push((10, "TAB_MOVED", before, serial(&run.root, &record)?.len()));
        scene::capture_settled(
            run,
            &mut c,
            "preview-reattached-native",
            &[(650, 250, [255, 0, 0]), (1100, 700, [255, 255, 255])],
        )?;

        resolution::activate(&run.root, &record, &mut c, 1)?;
        key(&mut c, &["ctrl", "n"])?;
        type_line(&mut c, "First independent window")?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["ctrl", "s"])?;
        fresh(run, &record, before, "DOOR_P11 NOTES SAVED")?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["ctrl", "shift", "n"])?;
        fresh(run, &record, before, "DOOR_P12 WINDOW_COMMITTED")?;
        type_line(&mut c, "Second independent window")?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["ctrl", "s"])?;
        fresh(run, &record, before, "DOOR_P11 NOTES SAVED")?;
        scene::capture_settled(run, &mut c, "notepad-two-native-windows", &[])?;
        key(&mut c, &["alt", "f4"])?;
        // Preserve every root app concurrently; open and close an independent
        // window of each remaining class without retiring its parent process.
        for index in [2, 3, 5] {
            resolution::activate(&run.root, &record, &mut c, index)?;
            let before = serial(&run.root, &record)?.len();
            key(&mut c, &["ctrl", "shift", "n"])?;
            fresh(run, &record, before, "DOOR_P12 WINDOW_COMMITTED")?;
            scene::capture_settled(run, &mut c, &format!("app-{index}-two-native-windows"), &[])?;
            key(&mut c, &["alt", "f4"])?;
        }
        resolution::activate(&run.root, &record, &mut c, 5)?;
        // Six apps, five services, Init's windowless search host and IME.
        let text = serial(&run.root, &record)?;
        let hosts = shutdown::search_hosts(&text).len()
            + shutdown::ime_hosts(&text).len()
            + shutdown::network_hosts(&text).len();
        fresh(
            run,
            &record,
            0,
            &format!("DOOR_MONITOR SETTLED processes={}", 11 + hosts),
        )?;
        key(&mut c, &["meta_l", "up"])?;
        scene::capture_settled(run, &mut c, "six-live-apps-monitor", &[])?;
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(30),
        )?;
        Ok(())
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = scene::capture_settled(run, &mut c, "multiwindow-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    result?;
    seal_serial(&run.root, &mut record)?;
    let sealed = serial(&run.root, &record)?;
    let retired = shutdown::retirement_commands(&sealed, &[1, 2, 3, 9, 10, 11])?;
    let actors = verify_actors(&sealed, &retired, &gestures)?;
    let raw = fs::read(&disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&raw)?;
    if journal.selected().committed {
        return Err("multiwindow journal not clean".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&raw)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("multiwindow catalog")?;
    let paths = catalog.paths()?;
    for expected in [
        b"First independent window\n".as_slice(),
        b"Second independent window\n".as_slice(),
        b"P3 2 2 255 255 0 0 0 255 0 0 0 255 255 255 255\n".as_slice(),
    ] {
        if paths
            .iter()
            .filter(|(p, _)| p.starts_with("/users/1001/notes/"))
            .filter(|(_, e)| {
                tree.files
                    .get(&format!("obj{:016x}", e.id))
                    .is_some_and(|f| f.bytes == expected)
            })
            .count()
            != 1
        {
            return Err("independent saved document missing/duplicated or altered source".into());
        }
    }
    run.check("multiwindow-native-owned-documents",json!({"session":record,"retirement":retired,"actors":actors,"disk_sha256":file_hash(&disk)?,"journal_clean":true,"independent_saved_documents":2,"source_image_unchanged":true,"owner_volume_used":false,"scope":"six concurrent app classes, separate same-process windows, actual Explorer/Preview tab detach and attach, distinct Notes saved bytes; not unlimited instances, full app providers or exact Windows fidelity"}));
    Ok(())
}

/// Require the complete Preview admission trace inside one authenticated native
/// incarnation. Exact event order rejects duplicate, replayed and foreign IDs.
fn verify_pressure(text: &str, retired: &Value) -> Result<Value, String> {
    let pid = retired["applications"]["10"]
        .as_u64()
        .ok_or("pressure Preview actor")?;
    let identities = retired["identities"]
        .as_array()
        .ok_or("pressure identities")?;
    let init = identities
        .iter()
        .find(|v| v["fields"]["manifest"] == 1)
        .and_then(|v| v["fields"]["pid"].as_u64())
        .ok_or("pressure Init")?;
    let actors: Vec<_> = identities
        .iter()
        .filter(|v| v["fields"]["pid"] == pid)
        .collect();
    if actors.len() != 1 {
        return Err("unique pressure incarnation required".into());
    }
    let actor = actors[0];
    let f = &actor["fields"];
    if f["manifest"] != 13
        || f["uid"] != 1001
        || f["exit"] != 0
        || ["instance", "session", "epoch"]
            .iter()
            .any(|k| f[*k].as_u64().unwrap_or(0) == 0)
    {
        return Err("pressure actor authority".into());
    }
    let end = actor["line"].as_u64().ok_or("pressure retirement line")? as usize;
    let launch = format!("DOOR_USER pid={init} DOOR_P12 APP_LAUNCHED app=10 pid={pid}");
    let launches: Vec<_> = text
        .lines()
        .enumerate()
        .filter(|(_, l)| *l == launch)
        .map(|(i, _)| i)
        .collect();
    if launches.len() != 1 || launches[0] >= end || end >= text.lines().count() {
        return Err("pressure launch/retirement boundary".into());
    }
    let created = observations(text, "DOOR_P12 WINDOW_CREATED")?;
    let own: Vec<_> = created.iter().filter(|v| v.0 == pid).collect();
    // The source's capture precedes the first new window; that window's
    // capture follows its creation. The budget derives from both.
    let first = own.get(1).ok_or("pressure new window")?.2;
    let (server, source_bytes) = preview_capture(text, first, false)?;
    let (window_server, window_bytes) = preview_capture(text, first, true)?;
    let admitted = pressure_budget(source_bytes, window_bytes)?;
    // Source, admitted windows, the refused one and the replacement.
    if window_server != server || own.len() != admitted + 3 {
        return Err(format!("{} pressure windows required", admitted + 3));
    }
    let ids: Vec<_> = own
        .iter()
        .map(|v| v.1.get("id").copied().ok_or("pressure window ID"))
        .collect::<Result<_, _>>()?;
    let mut owners = BTreeMap::new();
    for (p, fields, _, _) in &created {
        let id = fields.get("id").ok_or("creation ID")?;
        if fields.len() != 1 || owners.insert(*id, *p).is_some() {
            return Err("duplicate or malformed pressure creation".into());
        }
    }
    let mut events = Vec::new();
    for kind in [
        "WINDOW_CREATED",
        "WINDOW_COMMITTED",
        "WINDOW_RETIRED",
        "WINDOW_ADMISSION_REFUSED",
    ] {
        for (p, fields, byte, line) in observations(text, &format!("DOOR_P12 {kind}"))? {
            let involved = fields.values().any(|id| ids.contains(id));
            if p != pid {
                if involved || kind == "WINDOW_ADMISSION_REFUSED" {
                    return Err("foreign pressure event".into());
                }
                continue;
            }
            if line <= launches[0] || line >= end {
                return Err("pressure event outside incarnation".into());
            }
            events.push((byte, kind, fields));
        }
    }
    events.sort_by_key(|v| v.0);
    let one = |id| BTreeMap::from([("id".into(), id)]);
    let (n, refused, replacement) = (admitted, ids[admitted + 1], ids[admitted + 2]);
    let mut expected = Vec::new();
    for &id in &ids[..=n] {
        expected.push(("WINDOW_CREATED", one(id)));
        expected.push(("WINDOW_COMMITTED", one(id)));
    }
    expected.extend([
        ("WINDOW_CREATED", one(refused)),
        (
            "WINDOW_ADMISSION_REFUSED",
            BTreeMap::from([("from".into(), ids[n]), ("to".into(), refused)]),
        ),
        ("WINDOW_RETIRED", one(ids[n])),
        ("WINDOW_CREATED", one(replacement)),
        ("WINDOW_COMMITTED", one(replacement)),
        ("WINDOW_RETIRED", one(replacement)),
    ]);
    expected.extend(ids[..n].iter().rev().map(|&id| ("WINDOW_RETIRED", one(id))));
    if events
        .iter()
        .map(|(_, k, f)| (*k, f.clone()))
        .collect::<Vec<_>>()
        != expected
    {
        return Err("pressure create/commit/refusal/close lifetime order".into());
    }
    // The refusal is the server's capture pool turning away the refused
    // window's first frame, not a window count or another resource.
    let byte = |kind: &str, id: u64| {
        events
            .iter()
            .find(|(_, k, f)| *k == kind && f.values().any(|v| *v == id))
            .map(|e| e.0)
            .ok_or("pressure event offset")
    };
    let (from, to) = (
        byte("WINDOW_CREATED", refused)?,
        byte("WINDOW_ADMISSION_REFUSED", refused)?,
    );
    let lines = text.lines().count();
    let refusals: Vec<_> = observations(text, "DOOR_P12 CAPTURE RESOURCES")?
        .into_iter()
        .filter(|o| o.3 > launches[0] && o.3 < end.min(lines))
        .collect();
    if refusals.is_empty()
        || refusals.iter().any(|(p, fields, at, _)| {
            *p != server
                || *at <= from
                || *at >= to
                || fields.get("requested") != Some(&(window_bytes as u64))
        })
    {
        return Err("pressure refusal must be the server capture pool's".into());
    }
    Ok(
        json!({"identity":f,"created":ids,"admitted":n,"refused":refused,
        "replacement":replacement,"source_bytes":source_bytes,"window_bytes":window_bytes,
        "capture_refusals":refusals.len(),"launch_line":launches[0],"retirement_line":end,
        "ordered_window_events":events.len()}),
    )
}

/// The server's captures of app 10 (Preview): its pid and the bytes of the
/// first capture at or after serial byte `from` (`next`) or of the last one
/// before it.
fn preview_capture(text: &str, from: usize, next: bool) -> Result<(u64, usize), String> {
    let mut captures = Vec::new();
    let mut offset = 0;
    for raw in text.split_inclusive('\n') {
        if let Some((prefix, body)) = raw.trim_end().split_once(" DOOR_P12 CAPTURED app=10 ") {
            let pid: u64 = prefix
                .strip_prefix("DOOR_USER pid=")
                .and_then(|p| p.parse().ok())
                .ok_or("pressure capture actor")?;
            let field = |k: &str| {
                body.split_whitespace()
                    .find_map(|t| t.strip_prefix(k)?.strip_prefix('=')?.parse::<usize>().ok())
                    .ok_or("pressure capture size")
            };
            captures.push((offset, pid, field("width")? * field("height")? * 4));
        }
        offset += raw.len();
    }
    let pick = if next {
        captures.iter().find(|c| c.0 >= from)
    } else {
        captures.iter().rev().find(|c| c.0 < from)
    };
    pick.map(|c| (c.1, c.2))
        .ok_or_else(|| "pressure Preview capture".into())
}

/// New windows of `window` bytes the window server admits beside a source of
/// `source` bytes, replayed with its own constants. Registration needs a
/// window and buffer slot and room in the per-owner bytes (buffers plus
/// snapshots); the first capture then needs a pool cell when larger than
/// `LARGE_BYTES` (at most `RETAINED_CELLS` held) and snapshot room. The
/// scenario proves a refusal *after* registration, at the first capture by
/// the pool (which logs `CAPTURE RESOURCES`); any other first refusal is an
/// error, never a silently different proof.
fn pressure_budget(source: usize, window: usize) -> Result<usize, String> {
    use door_window::capture::{CELL_BYTES, LARGE_BYTES, RETAINED_CELLS};
    let limits = door_window::policy::limits();
    let quota = limits.bytes_per_owner.min(limits.total_bytes);
    if source > CELL_BYTES || window > CELL_BYTES {
        return Err(format!(
            "pressure sizes {source}/{window} exceed a capture cell"
        ));
    }
    // The source's buffer and snapshot, and its cell.
    let (mut owner, mut cells) = (2 * source, usize::from(source > LARGE_BYTES));
    for k in 1.. {
        if k + 1 > limits.windows_per_owner.min(limits.buffers_per_owner) || owner + window > quota
        {
            return Err(format!(
                "pressure: new window {k} would be refused at registration, not at its first capture"
            ));
        }
        if window > LARGE_BYTES && cells >= RETAINED_CELLS {
            return if k > 1 {
                Ok(k - 1)
            } else {
                Err("pressure: no window admitted before the refusal".into())
            };
        }
        if owner + 2 * window > quota {
            return Err(format!(
                "pressure: new window {k} would be refused by the snapshot quota, not the pool"
            ));
        }
        owner += 2 * window;
        cells += usize::from(window > LARGE_BYTES);
    }
    unreachable!()
}

/// Window-server capture budget at 3x: windows open until the pool refuses a
/// first frame after registration, then a replacement commits after a close.
pub(super) fn pressure(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("multiwindow-pressure-private.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, "3840x2160")?;
    let result = (|| -> Result<(), String> {
        let mut c = connection(&record)?;
        let mut p = (1920, 1080);
        resolution::activate(&run.root, &record, &mut c, 1)?;
        type_line(&mut c, "P3 2 2 255 255 0 0 0 255 0 0 0 255 255 255 255")?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["ctrl", "s"])?;
        fresh(run, &record, before, "DOOR_P11 NOTES SAVED")?;
        key(&mut c, &["ctrl", "w"])?;
        resolution::activate(&run.root, &record, &mut c, 0)?;
        key(&mut c, &["ctrl", "l"])?;
        type_line(&mut c, r"C:\Users\1001\notes")?;
        key(&mut c, &["home"])?;
        key(&mut c, &["f2"])?;
        key(&mut c, &["ctrl", "a"])?;
        let before = serial(&run.root, &record)?.len();
        type_line(&mut c, "pressure-colors.ppm")?;
        fresh(run, &record, before, "FILES RENAME COMMITTED")?;
        key(&mut c, &["ctrl", "w"])?;
        resolution::activate(&run.root, &record, &mut c, 4)?;
        key(&mut c, &["meta_l", "up"])?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["ctrl", "o"])?;
        type_line(&mut c, r"C:\Users\1001\notes\pressure-colors.ppm")?;
        fresh(run, &record, before, "image=1")?;
        let probes = [
            (400 * 3, 250 * 3, [255, 0, 0]),
            (700 * 3, 500 * 3, [255, 255, 255]),
        ];
        scene::capture_settled(run, &mut c, "pressure-source-before", &probes)?;
        // Restore the source: maximized, its buffer and snapshot leave no
        // room to register the window the pool would refuse at 3x, so the
        // owner quota would refuse it at creation instead (`pressure_budget`).
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "down"])?;
        fresh(run, &record, before, "DOOR_P12 CAPTURED app=10 ")?;
        // Open windows until the server's real capture budget is exceeded:
        // how many it admits follows from its constants and the sizes it
        // actually captured (the restored source and the first new window).
        let start = serial(&run.root, &record)?.len();
        key(&mut c, &["ctrl", "shift", "n"])?;
        fresh(run, &record, start, "DOOR_P12 WINDOW_COMMITTED")?;
        let text = serial(&run.root, &record)?;
        let admitted = pressure_budget(
            preview_capture(&text, start, false)?.1,
            preview_capture(&text, start, true)?.1,
        )?;
        for _ in 1..admitted {
            let before = serial(&run.root, &record)?.len();
            key(&mut c, &["ctrl", "shift", "n"])?;
            fresh(run, &record, before, "DOOR_P12 WINDOW_COMMITTED")?;
        }
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["ctrl", "shift", "n"])?;
        fresh(run, &record, before, "DOOR_P12 WINDOW_ADMISSION_REFUSED")?;
        scene::capture_settled(run, &mut c, "pressure-admission-refused", &[])?;
        // Closing one admitted window frees its cell: a replacement commits.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        fresh(run, &record, before, "DOOR_P12 WINDOW_RETIRED")?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["ctrl", "shift", "n"])?;
        fresh(run, &record, before, "DOOR_P12 WINDOW_COMMITTED")?;
        scene::capture_settled(run, &mut c, "pressure-after-recovery", &[])?;
        // The replacement and the other admitted windows cover the source.
        for _ in 0..admitted {
            let before = serial(&run.root, &record)?.len();
            key(&mut c, &["alt", "f4"])?;
            fresh(run, &record, before, "DOOR_P12 WINDOW_RETIRED")?;
        }
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "up"])?;
        fresh(run, &record, before, "DOOR_P12 CAPTURED app=10 width=3840 ")?;
        scene::capture_settled(run, &mut c, "pressure-source-retained", &probes)?;
        key(&mut c, &["alt", "f4"])?;
        // Launch Monitor from Start's All section (Category view): a pointer
        // launch through a category card, independent of pinned navigation.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l"])?;
        fresh(run, &record, before, "SCENE_PRESENTED app=0 overlay=1")?;
        // With the search host, Start > Recent lists the user's recent files
        // above All; place the category card with the same item count.
        let recent = if shutdown::search_hosts(&serial(&run.root, &record)?).is_empty() {
            0
        } else {
            // The shell logs the Recent count only when it changes: give the
            // list time to arrive, then use the newest count in the session.
            thread::sleep(Duration::from_secs(1));
            fresh(run, &record, 0, "DOOR_P12 START_RECENT count=")?;
            serial(&run.root, &record)?
                .lines()
                .filter_map(|l| l.split_once("DOOR_P12 START_RECENT count="))
                .filter_map(|(_, n)| n.trim().parse::<usize>().ok())
                .next_back()
                .ok_or("Start Recent count")?
        };
        let mut start = door_window::start::Start::default();
        start.recent = (0..recent)
            .map(|_| door_window::start::FileResult {
                folder: false,
                name: "x".into(),
                path: String::new(),
            })
            .collect();
        let geometry = start.geometry(door_window::taskbar::Layout::new(1280, 720));
        let monitor = door_window::start::Target::CategoryApp(0, 2);
        if start.app(monitor) != Some(door_window::apps::App::Monitor) {
            return Err("Utilities & tools card no longer holds Monitor third".into());
        }
        scene::capture_settled(run, &mut c, "launcher-all-categories-3x", &[])?;
        let cell = geometry.card_cell(geometry.card(0, 0), 2);
        let before = serial(&run.root, &record)?.len();
        click(
            &mut c,
            &mut p,
            (cell.x() + cell.width() as i32 / 2) * 3,
            (cell.y() + cell.height() as i32 / 2) * 3,
        )?;
        resolution::wait_activation(&run.root, &record, 11, before)?;
        key(&mut c, &["alt", "f4"])?;
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(30),
        )?;
        Ok(())
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = scene::capture_settled(run, &mut c, "pressure-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    result?;
    seal_serial(&run.root, &mut record)?;
    let text = serial(&run.root, &record)?;
    let retired = shutdown::retirement_commands(&text, &[2, 1, 10, 11])?;
    let proof = verify_pressure(&text, &retired)?;
    let raw = fs::read(&disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&raw)?;
    if journal.selected().committed {
        return Err("pressure journal not clean".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&raw)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("pressure catalog")?;
    let paths = catalog.paths()?;
    let source = paths
        .get("/users/1001/notes/pressure-colors.ppm")
        .ok_or("pressure source")?;
    if tree
        .files
        .get(&format!("obj{:016x}", source.id))
        .is_none_or(|f| f.bytes != b"P3 2 2 255 255 0 0 0 255 0 0 0 255 255 255 255\n")
    {
        return Err("pressure altered source bytes".into());
    }
    run.check("multiwindow-native-quota-refusal-recovery",json!({"session":record,"retirement":retired,"actors":proof,"source_unchanged":true,"journal_clean":true,"disk_sha256":file_hash(&disk)?,"owner_volume_used":false,"scope":"Actual 3x window-server capture budget (windows opened until the capture pool refuses a first frame after registration, count derived from its constants), rollback, successful replacement after close, source pixels and durable bytes preserved; provisional edit suppression is covered by source/host tests, not this native workflow; not native1MP codec or physical OOM qualification"}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pressure_proof_rejects_reordered_replayed_and_foreign_lifetimes() {
        // 3x: restored source and new windows 2688x1572 (two admitted).
        let lines = [
            "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED app=10 pid=8",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_CREATED id=80 title=Preview",
            "DOOR_USER pid=5 DOOR_P12 CAPTURED app=10 width=2688 height=1572 epoch=3",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_COMMITTED id=80",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_CREATED id=81 title=Preview",
            "DOOR_USER pid=5 DOOR_P12 CAPTURED app=10 width=2688 height=1572 epoch=1",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_COMMITTED id=81",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_CREATED id=82 title=Preview",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_COMMITTED id=82",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_CREATED id=83 title=Preview",
            "DOOR_USER pid=5 DOOR_P12 CAPTURE RESOURCES requested=16902144 used=9 largest=9",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_ADMISSION_REFUSED from=82 to=83",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_RETIRED id=82",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_CREATED id=84 title=Preview",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_COMMITTED id=84",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_RETIRED id=84",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_RETIRED id=81",
            "DOOR_USER pid=8 DOOR_P12 WINDOW_RETIRED id=80",
            "kernel retired identity",
        ];
        let retired = json!({"applications":{"10":8},"identities":[
            {"fields":{"manifest":1,"pid":1}},
            {"line":18,"fields":{"manifest":13,"pid":8,"instance":99,"uid":1001,"session":1,"epoch":1,"exit":0}}
        ]});
        let proof = verify_pressure(&lines.join("\n"), &retired).unwrap();
        assert_eq!(
            (&proof["admitted"], &proof["refused"]),
            (&json!(2), &json!(83))
        );
        for (a, b) in [
            (1, 3),
            (6, 8),
            (11, 12),
            (14, 15),
            (15, 16),
            (0, 1),
            (10, 11),
        ] {
            let mut bad = lines;
            bad.swap(a, b);
            assert!(verify_pressure(&bad.join("\n"), &retired).is_err());
        }
        for (at, replacement) in [
            (
                11,
                "DOOR_USER pid=9 DOOR_P12 WINDOW_ADMISSION_REFUSED from=82 to=83",
            ),
            (15, "DOOR_USER pid=8 DOOR_P12 WINDOW_RETIRED id=82"),
            (14, "DOOR_USER pid=8 DOOR_P12 WINDOW_COMMITTED id=83"),
            (
                13,
                "DOOR_USER pid=8 DOOR_P12 WINDOW_CREATED id=80 title=Preview",
            ),
            // Not the pool's refusal of the refused window's frame.
            (
                10,
                "DOOR_USER pid=5 DOOR_P12 CAPTURE RESOURCES requested=20736000 used=9 largest=9",
            ),
            (
                10,
                "DOOR_USER pid=6 DOOR_P12 CAPTURE RESOURCES requested=16902144 used=9 largest=9",
            ),
            (10, "DOOR_USER pid=5 DOOR_P12 BAR FAST transferred_rows=48"),
            // A window below `LARGE_BYTES` never takes a cell.
            (
                5,
                "DOOR_USER pid=5 DOOR_P12 CAPTURED app=10 width=1000 height=600 epoch=1",
            ),
        ] {
            let mut bad = lines;
            bad[at] = replacement;
            assert!(verify_pressure(&bad.join("\n"), &retired).is_err());
        }
        let mut bad = retired.clone();
        bad["identities"][1]["line"] = json!(9);
        assert!(verify_pressure(&lines.join("\n"), &bad).is_err());
        bad = retired.clone();
        bad["identities"][1]["fields"]["manifest"] = json!(12);
        assert!(verify_pressure(&lines.join("\n"), &bad).is_err());
    }
    #[test]
    fn pressure_budget_follows_the_server_capture_pool() {
        use door_window::capture::{CELL_BYTES, LARGE_BYTES, RETAINED_CELLS};
        let (restored, maximized) = (2688 * 1572 * 4, 3840 * 2016 * 4);
        // Restored source: the pool refuses the window after the admitted two.
        assert_eq!(pressure_budget(restored, restored), Ok(RETAINED_CELLS - 1));
        // Maximized source: the third window cannot even register (the
        // failure this redesign replaced), and the old 960x600 placement hit
        // the snapshot quota first; neither is the pool refusal proved here.
        assert!(
            pressure_budget(maximized, restored)
                .unwrap_err()
                .contains("registration")
        );
        assert!(
            pressure_budget(maximized, 2880 * 1800 * 4)
                .unwrap_err()
                .contains("quota")
        );
        // Windows that never take a cell run into the window limit instead.
        assert!(pressure_budget(restored, LARGE_BYTES).is_err());
        assert!(pressure_budget(restored, CELL_BYTES + 1).is_err());
    }
    #[test]
    fn canonical_window_observations_reject_prefixes_duplicate_fields_and_zero_ids() {
        assert!(
            records(
                "DOOR_USER pid=6 DOOR_P12 WINDOW_CREATED id=9 title=Files\n",
                "DOOR_P12 WINDOW_CREATED"
            )
            .is_ok()
        );
        for line in [
            "DOOR_USER pid=06 DOOR_P12 TAB_MOVED from=9 to=10",
            "prefix DOOR_USER pid=6 DOOR_P12 TAB_MOVED from=9 to=10",
            "DOOR_USER pid=6 DOOR_P12 TAB_MOVED from=9 from=10",
            "DOOR_USER pid=6 DOOR_P12 TAB_MOVED from=9 to=0",
        ] {
            assert!(records(line, "DOOR_P12 TAB_MOVED").is_err());
        }
    }
    #[test]
    fn window_proof_requires_each_apps_detach_attach_commit_and_final_incarnation() {
        let mut text = String::new();
        let mut apps = serde_json::Map::new();
        let mut identities = vec![json!({"fields":{"manifest":1,"pid":1}})];
        for (i, command) in [1, 2, 3, 9, 10, 11].into_iter().enumerate() {
            let pid = i + 6;
            apps.insert(command.to_string(), json!(pid));
            text.push_str(&format!(
                "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED app={command} pid={pid}\n"
            ));
            for id in [pid * 100, pid * 100 + 1] {
                text.push_str(&format!("DOOR_USER pid={pid} DOOR_P12 WINDOW_CREATED id={id} title=App\nDOOR_USER pid={pid} DOOR_P12 WINDOW_COMMITTED id={id}\n"));
            }
            identities.push(json!({"line":999,"fields":{"manifest":if command==10{13}else if command==11{14}else{12},"pid":pid,"instance":pid+100,"uid":1001,"session":1,"epoch":1,"exit":0}}));
        }
        let mut gestures = Vec::new();
        for (command, kind) in GESTURES {
            let pid = apps[&command.to_string()].as_u64().unwrap();
            let before = text.len();
            let (a, b) = if kind == "TAB_DETACHED" {
                (pid * 100, pid * 100 + 1)
            } else {
                (pid * 100 + 1, pid * 100)
            };
            text.push_str(&format!(
                "DOOR_USER pid={pid} DOOR_P12 {kind} from={a} to={b}{}\n",
                if kind == "TAB_DETACHED" { " tab=9" } else { "" }
            ));
            gestures.push((command, kind, before, text.len()));
        }
        for identity in identities.iter_mut().skip(1) {
            identity["line"] = json!(text.lines().count());
            text.push_str("kernel retired identity\n");
        }
        let retired = json!({"applications":apps,"identities":identities});
        assert!(verify_actors(&text, &retired, &gestures).is_ok());
        let mut wrong = gestures.clone();
        wrong[2].0 = 1;
        assert!(verify_actors(&text, &retired, &wrong).is_err());
        wrong = gestures.clone();
        wrong[0].1 = "TAB_MOVED";
        assert!(verify_actors(&text, &retired, &wrong).is_err());
        let mut bad = retired.clone();
        bad["identities"][1]["fields"]["manifest"] = json!(13);
        assert!(verify_actors(&text, &bad, &gestures).is_err());
        bad = retired.clone();
        bad["identities"][1]["line"] = json!(1);
        assert!(verify_actors(&text, &bad, &gestures).is_err());
        let missing = text.replace(
            "DOOR_USER pid=10 DOOR_P12 WINDOW_COMMITTED id=1001",
            "DOOR_USER pid=10 DOOR_P12 WINDOW_COMMITTED id=1000",
        );
        assert!(verify_actors(&missing, &retired, &gestures).is_err());
        assert!(verify_actors(&text, &retired, &gestures[..3]).is_err());
    }
}
