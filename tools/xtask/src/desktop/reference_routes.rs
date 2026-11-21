//! Bind observed GUI routes to the launched and finally retired native actor.
use super::*;
pub(super) fn validate(
    text: &str,
    retired: &Value,
    command: u32,
    manifest: u64,
    marker: &str,
    visits: &[(u32, usize)],
) -> Result<Value, String> {
    let pid = retired["applications"][command.to_string()]
        .as_u64()
        .ok_or("route app PID")?;
    let identity = retired["identities"]
        .as_array()
        .ok_or("route identities")?
        .iter()
        .find(|p| p["fields"]["pid"] == pid)
        .ok_or("route retired actor")?;
    if identity["fields"]["manifest"] != manifest {
        return Err("route manifest".into());
    }
    let end = identity["line"].as_u64().ok_or("route retirement line")? as usize;
    let launch_suffix = format!(" DOOR_P12 APP_LAUNCHED app={command} pid={pid}");
    let launch = text
        .lines()
        .position(|l| l.ends_with(&launch_suffix))
        .ok_or("route launch")?;
    if launch >= end || visits.is_empty() || visits.windows(2).any(|v| v[0].1 >= v[1].1) {
        return Err("route ordering".into());
    }
    let prefix = format!("DOOR_USER pid={pid} {marker} id=");
    let mut offset = 0;
    let mut points = Vec::new();
    let mut end_offset = None;
    let mut launch_end = 0;
    for (i, raw) in text.split_inclusive('\n').enumerate() {
        let line = raw.trim_end_matches(['\r', '\n']);
        if i == launch {
            launch_end = offset + raw.len();
        }
        if i == end {
            end_offset = Some(offset);
        }
        if line.contains(marker) {
            let id = line
                .strip_prefix(&prefix)
                .ok_or("foreign or wrapped route marker")?;
            let number = id.parse::<u32>().map_err(|_| "route number")?;
            if number.to_string() != id || i <= launch || i >= end {
                return Err("noncanonical route actor/lifetime".into());
            }
            points.push((number, offset, i));
        }
        offset += raw.len();
    }
    let end_offset = end_offset.ok_or("route retirement offset")?;
    let mut observed = Vec::new();
    for (i, (id, before)) in visits.iter().enumerate() {
        let after = visits.get(i + 1).map_or(end_offset, |v| v.1);
        if *before < launch_end || *before >= after || after > end_offset {
            return Err("route fresh byte window".into());
        }
        let matches: Vec<_> = points
            .iter()
            .filter(|p| p.1 >= *before && p.1 < after)
            .collect();
        if matches.len() != 1 || matches[0].0 != *id {
            return Err("missing, duplicate, or wrong fresh route".into());
        }
        observed.push(json!({"page":id,"before_byte":before,"line":matches[0].2}));
    }
    Ok(
        json!({"actor":identity["fields"],"visits":observed,"scope":"Fresh native input routes bound to exact app actor and retirement; not pixel fidelity or unavailable-provider qualification"}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_require_the_exact_actor_lifetime_and_fresh_click_window() {
        let text = "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED app=11 pid=6\nDOOR_USER pid=6 DOOR_MONITOR PAGE id=0\nDOOR_USER pid=6 DOOR_MONITOR PAGE id=20\nDOOR_P08 PROCESS manifest=14 pid=6\n";
        let retired = json!({"applications":{"11":6},"identities":[{"line":3,"fields":{"manifest":14,"pid":6,"instance":8}}]});
        let a = text.find("DOOR_USER pid=6").unwrap();
        let b = text
            .find("DOOR_USER pid=6 DOOR_MONITOR PAGE id=20")
            .unwrap();
        let visits = [(0, a), (20, b)];
        assert!(validate(text, &retired, 11, 14, "DOOR_MONITOR PAGE", &visits).is_ok());
        for bad in [
            text.replace("pid=6 DOOR_MONITOR", "pid=7 DOOR_MONITOR"),
            text.replace("PAGE id=20", "PAGE id=020"),
            text.replace("PAGE id=20", "prefix PAGE id=20"),
            text.replace("PAGE id=20", "PAGE id=1"),
        ] {
            assert!(validate(&bad, &retired, 11, 14, "DOOR_MONITOR PAGE", &visits).is_err());
        }
        assert!(
            validate(
                text,
                &retired,
                11,
                14,
                "DOOR_MONITOR PAGE",
                &[(0, a + 1), (20, b)]
            )
            .is_err()
        );
        assert!(
            validate(
                text,
                &retired,
                11,
                14,
                "DOOR_MONITOR PAGE",
                &[(20, a), (20, b)]
            )
            .is_err()
        );
    }
}
