//! Validate complete bounded paint aggregates against kernel-owned retirement.
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MARKER: &str = "DOOR_P12 MOTION ";
const REUSE: &str = "DOOR_P12 MOVE_REUSE ";

pub(super) fn present(text: &str) -> bool {
    text.contains(MARKER) || text.contains(REUSE)
}

pub(super) fn validate_saved(text: &str, saved: &Value) -> Result<(), String> {
    if !present(text) && saved.is_null() {
        return Ok(());
    }
    if saved.is_null() || validate(text)? != *saved {
        return Err(
            "retained motion aggregate missing or differs from sealed native serial".into(),
        );
    }
    Ok(())
}

pub(super) fn validate(text: &str) -> Result<Value, String> {
    let canonical = super::shutdown::canonical_for(text, 8)?;
    parse(
        text,
        canonical["window"]["pid"]
            .as_u64()
            .ok_or("motion Window pid")?,
        canonical["client_exit_line"]
            .as_u64()
            .ok_or("motion client exit")?,
        canonical["window_exit_line"]
            .as_u64()
            .ok_or("motion Window exit")?,
    )
}

fn number(value: &str, limit: u64) -> Result<u64, String> {
    let n = u64::from_str_radix(value, 16).map_err(|_| "invalid motion counter")?;
    if format!("{n:x}") != value || n >= limit {
        return Err("noncanonical, overflowing or saturated motion counter".into());
    }
    Ok(n)
}
fn group(payload: &str, dragging: usize) -> Result<Value, String> {
    let mut fields = BTreeMap::new();
    for token in payload
        .strip_prefix(MARKER)
        .ok_or("motion prefix")?
        .split(' ')
    {
        let (key, value) = token.split_once('=').ok_or("motion field")?;
        if fields.insert(key, value).is_some() {
            return Err("duplicate motion field".into());
        }
    }
    if fields.keys().copied().collect::<Vec<_>>()
        != [
            "b", "c", "cut", "d", "f", "h", "k", "m", "ms", "n", "p", "t", "v", "x",
        ]
        || fields["v"] != "1"
        || fields["ms"] != door_abi::platform::MONOTONIC_TICK_MILLIS.to_string()
        || fields["x"] != "1"
        || fields["cut"] != "p"
        || fields["d"] != dragging.to_string()
    {
        return Err("motion version, fields, cutoff or group differs".into());
    }
    let scalar = |key: &str, limit| number(fields[key], limit);
    let n = scalar("n", u32::MAX as u64)?;
    let compose = scalar("c", u64::MAX)?;
    let transfer = scalar("t", u64::MAX)?;
    let bytes = scalar("b", u64::MAX)?;
    let calls = scalar("k", u64::MAX)?;
    let maximum = scalar("m", u32::MAX as u64)?;
    let failed = scalar("f", u32::MAX as u64)?;
    let vector = |key: &str, count| -> Result<Vec<u64>, String> {
        let values = fields[key]
            .split(',')
            .map(|v| number(v, u32::MAX as u64))
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != count {
            return Err("motion vector extent".into());
        }
        Ok(values)
    };
    let histogram = vector("h", 7)?;
    let paths = vector("p", 4)?;
    let total = compose
        .checked_add(transfer)
        .ok_or("motion time overflow")?;
    let max_total = n.checked_mul(maximum).ok_or("motion maximum overflow")?;
    let upper_bytes = calls
        .checked_mul(door_abi::graphics::MAX_COPY_BYTES as u64)
        .ok_or("motion byte bound overflow")?;
    if n == 0
        || failed != 0
        || histogram.iter().sum::<u64>() != n
        || paths.iter().sum::<u64>() != n
        || total < maximum
        || total > max_total
        || bytes % 4 != 0
        || bytes > upper_bytes
        || bytes < 4 * calls
        || calls == 0
    {
        return Err("motion count, failure, time or transfer contradiction".into());
    }
    let low = [0, 1, 2, 3, 4, 8, 16];
    let high = [0, 1, 2, 3, 7, 15, maximum].map(|bound| bound.min(maximum));
    let highest = histogram
        .iter()
        .rposition(|n| *n > 0)
        .ok_or("motion empty histogram")?;
    let weighted = |bounds: [u64; 7]| -> Result<u64, String> {
        histogram
            .iter()
            .zip(bounds)
            .try_fold(0u64, |sum, (count, bound)| {
                count
                    .checked_mul(bound)
                    .and_then(|v| sum.checked_add(v))
                    .ok_or_else(|| "motion histogram overflow".into())
            })
    };
    let minimum_total = weighted(low)?
        .checked_add(maximum.saturating_sub(low[highest]))
        .ok_or("motion attained maximum overflow")?;
    if maximum < low[highest]
        || maximum > high[highest]
        || total < minimum_total
        || total > weighted(high)?
    {
        return Err("motion histogram timing contradiction".into());
    }
    Ok(
        json!({"dragging":dragging == 1,"frames":n,"compose_ticks":compose,
        "transfer_ticks":transfer,"accepted_bytes":bytes,"attempted_calls":calls,
        "maximum_work_ticks":maximum,"failed":failed,"histogram":histogram,"paths":paths}),
    )
}

fn parse(text: &str, pid: u64, after: u64, before: u64) -> Result<Value, String> {
    let mut groups = Vec::new();
    let mut lines = Vec::new();
    let mut diagnostic_bytes = 0usize;
    let mut summary_bytes = 0usize;
    let mut reused = None;
    let mut display_ready = None;
    let ready_marker =
        format!("DOOR_USER pid={pid} DOOR_P11 DISPLAY READY cpu-copy=1 shared-readonly=1");
    for (i, line) in text.lines().enumerate() {
        if line == ready_marker && display_ready.replace(i).is_some() {
            return Err("duplicate Window display readiness".into());
        }
        if line.contains(REUSE) {
            let payload = line
                .strip_prefix(&format!("DOOR_USER pid={pid} {REUSE}"))
                .ok_or("move reuse actor or prefix")?;
            if reused.is_some()
                || i as u64 + 1 >= before
                || payload.len() > 160
                || display_ready.is_none_or(|ready| ready >= i)
                || !groups.is_empty()
            {
                return Err("move reuse count, length or retirement order".into());
            }
            let fields = payload.split(' ').collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err("move reuse fields".into());
            }
            let mut values = Vec::new();
            for (token, name) in fields
                .into_iter()
                .zip(["serial", "pixels", "bytes", "calls"])
            {
                let n = token
                    .strip_prefix(&format!("{name}="))
                    .ok_or("move reuse field name")?;
                let value = n.parse::<u64>().map_err(|_| "move reuse counter")?;
                if value == 0 || value.to_string() != n {
                    return Err("move reuse canonical positive counter".into());
                }
                values.push(value);
            }
            if values[1] > 3840 * 2160
                || values[2] % 4 != 0
                || values[1] * 4 > values[2]
                || values[3]
                    .checked_mul(16384)
                    .is_none_or(|limit| values[2] > limit)
                || values[3]
                    .checked_mul(4)
                    .is_none_or(|minimum| values[2] < minimum)
            {
                return Err("move reuse geometry or transfer counters".into());
            }
            reused = Some(
                json!({"line_number":i+1,"serial":values[0],"pixels":values[1],
                "accepted_bytes":values[2],"attempted_calls":values[3]}),
            );
        }
        let user = line
            .strip_prefix("DOOR_USER pid=")
            .and_then(|s| s.split_once(' '));
        if let Some((actor, payload)) = user {
            if actor == pid.to_string() {
                diagnostic_bytes = diagnostic_bytes
                    .checked_add(payload.len() + 1)
                    .ok_or("motion diagnostic length overflow")?;
            }
            if !line.contains(MARKER) {
                continue;
            }
            if actor != pid.to_string()
                || !payload.starts_with(MARKER)
                || payload.len() + 1 > 256
                || groups.len() >= 2
                || (i as u64) < after
                || i as u64 + 1 >= before
            {
                return Err("motion actor, record size, count or retirement order".into());
            }
            groups.push(group(payload, groups.len())?);
            lines.push(i + 1);
            summary_bytes += payload.len() + 1;
        } else if line.contains(MARKER) {
            return Err("unwrapped motion diagnostic".into());
        }
    }
    if groups.len() != 2 || summary_bytes > 512 || diagnostic_bytes > 16384 {
        return Err("motion summaries missing or diagnostic budget exceeded".into());
    }
    let mut result = json!({"schema_version":1,"window_pid":pid,"cutoff":"completed paint scopes before first final power attempt",
        "line_numbers":lines,"tick_millis":door_abi::platform::MONOTONIC_TICK_MILLIS,
        "groups":groups,"summary_source_bytes":summary_bytes,"window_diagnostic_source_bytes":diagnostic_bytes,
        "scope":"scene/caption/icons/bar; excludes cursor-only updates, idle and final cursor/logging; includes input serviced during transfer",
        "fps_claim":false,"vblank_observed":false});
    if let Some(reused) = reused {
        result["move_reuse"] = reused;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> String {
        let record = "DOOR_USER pid=5 DOOR_P12 MOTION v=1 ms=10 x=1 cut=p d=D n=4 c=2 t=2 b=100 k=4 m=1 f=0 h=0,4,0,0,0,0,0 p=1,1,1,1\n";
        format!(
            "prelude\n{}{}retired\n",
            record.replace("d=D", "d=0"),
            record.replace("d=D", "d=1")
        )
    }
    fn check(text: &str) -> Result<Value, String> {
        parse(text, 5, 1, 4)
    }
    #[test]
    fn exact_groups_retain_all_paths_and_kernel_order() {
        let v = check(&fixture()).unwrap();
        assert_eq!(v["groups"][1]["frames"], 4);
        assert_eq!(v["groups"][1]["accepted_bytes"], 256);
        assert_eq!(v["line_numbers"], json!([2, 3]));
        assert_eq!(v["fps_claim"], false);
    }
    #[test]
    fn rejects_missing_duplicate_foreign_unwrapped_and_out_of_order_groups() {
        let text = fixture();
        for changed in [
            text.replace("d=1", "d=0"),
            text.replace("pid=5", "pid=6"),
            text.replace("DOOR_USER pid=5 ", ""),
            text.lines().take(2).collect::<Vec<_>>().join("\n"),
            format!("{text}{text}"),
            text.replace("d=0", "d=01"),
        ] {
            assert!(check(&changed).is_err(), "{changed}");
        }
        assert!(parse(&text, 5, 2, 4).is_err());
        assert!(parse(&text, 5, 1, 3).is_err());
    }
    #[test]
    fn rejects_forged_and_inconsistent_counters() {
        let text = fixture();
        for (old, new) in [
            ("v=1", "v=2"),
            ("ms=10", "ms=1"),
            ("cut=p", "cut=e"),
            ("x=1", "x=2"),
            ("n=4", "n=04"),
            ("n=4", "n=ffffffff"),
            ("c=2", "c=ffffffffffffffff"),
            ("c=2", "c=FFFFFFFFFFFFFFFF"),
            ("n=4", "n=4 n=4"),
            ("n=4", "n=4 z=0"),
            ("n=4", "n=5"),
            ("f=0", "f=1"),
            ("t=2", "t=3"),
            ("m=1", "m=2"),
            ("b=100", "b=101"),
            ("b=100", "b=40004"),
            ("k=4", "k=0"),
            ("p=1,1,1,1", "p=4"),
            ("h=0,4,0,0,0,0,0", "h=0,0,0,4,0,0,0"),
            ("n=4", "n=-1"),
            ("n=4", "n=10000000000000000"),
        ] {
            assert!(check(&text.replace(old, new)).is_err(), "{old} => {new}");
        }
    }
    #[test]
    fn rejects_diagnostic_exhaustion_and_oversized_source_records() {
        let text = fixture();
        assert!(
            parse(
                &format!("DOOR_USER pid=5 {}\n{text}", "a".repeat(16384)),
                5,
                2,
                5
            )
            .is_err()
        );
        assert!(check(&text.replace("x=1", &format!("x=1 {}", " ".repeat(256)))).is_err());
    }
    #[test]
    fn retained_summary_cannot_be_stripped_from_new_native_evidence() {
        assert!(validate_saved(&fixture(), &Value::Null).is_err());
        assert!(validate_saved("historical serial without aggregates\n", &Value::Null).is_ok());
        assert!(
            validate_saved(
                "historical serial without aggregates\n",
                &json!({"forged":true})
            )
            .is_err()
        );
    }
    #[test]
    fn histogram_maximum_must_be_attained_and_bound_every_other_frame() {
        let record = "DOOR_P12 MOTION v=1 ms=10 x=1 cut=p d=0 n=2 c=0 t=64 b=8 k=2 m=64 f=0 h=0,0,0,0,0,0,2 p=2,0,0,0";
        // One100tick frame and another >=16tick frame cannot total100.
        assert!(group(record, 0).is_err());
        assert!(group(&record.replace("t=64", "t=74"), 0).is_ok());
        let record = "DOOR_P12 MOTION v=1 ms=10 x=1 cut=p d=0 n=2 c=0 t=8 b=8 k=2 m=5 f=0 h=0,1,0,0,1,0,0 p=2,0,0,0";
        // One1tick frame and a maximum5tick frame must total6, not8.
        assert!(group(record, 0).is_err());
        assert!(group(&record.replace("t=8", "t=6"), 0).is_ok());
    }
    #[test]
    fn reuse_proof_is_optional_for_history_but_actor_bound_and_recomputed_when_present() {
        let marker = "DOOR_USER pid=5 DOOR_P12 MOVE_REUSE serial=40 pixels=100 bytes=512 calls=2\n";
        let ready = "DOOR_USER pid=5 DOOR_P11 DISPLAY READY cpu-copy=1 shared-readonly=1\n";
        let text = format!("{ready}{marker}{}", fixture());
        let parsed = parse(&text, 5, 3, 6).unwrap();
        assert_eq!(parsed["move_reuse"]["pixels"], 100);
        for changed in [
            marker.replace("pid=5", "pid=6"),
            marker.replace("serial=40", "serial=040"),
            marker.replace("pixels=100", "pixels=0"),
            marker.replace("pixels=100", "pixels=99999999"),
            marker.replace("bytes=512", "bytes=396"),
            marker.replace("bytes=512", "bytes=513"),
            marker.replace("calls=2", "calls=99999999"),
            marker.replace("DOOR_USER pid=5 ", ""),
            format!("{marker}{marker}"),
        ] {
            assert!(parse(&format!("{ready}{changed}{}", fixture()), 5, 3, 6).is_err());
        }
        assert!(validate_saved(marker, &Value::Null).is_err());
        assert!(parse(&format!("{marker}{ready}{}", fixture()), 5, 3, 6).is_err());
        let late = fixture().replace("retired", &format!("{marker}retired"));
        assert!(parse(&format!("{ready}{late}"), 5, 2, 6).is_err());
        assert!(check(&fixture()).unwrap().get("move_reuse").is_none());
        let mut stripped = parsed;
        stripped.as_object_mut().unwrap().remove("move_reuse");
        assert_ne!(parse(&text, 5, 3, 6).unwrap(), stripped);
    }
}
