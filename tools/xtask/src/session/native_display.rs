//! Read adapter mode configuration, without screen capture or host UI control.
use super::*;
use std::collections::BTreeSet;

pub(crate) const MODE_QUERY: &str = "$modes=@(Get-CimInstance Win32_VideoController -ErrorAction Stop | ForEach-Object { [pscustomobject]@{width=$_.CurrentHorizontalResolution;height=$_.CurrentVerticalResolution} }); ConvertTo-Json -Compress -InputObject $modes";
pub(crate) fn single_mode(text: &str) -> Option<String> {
    if text.len() > 4096 {
        return None;
    }
    let rows: Value = serde_json::from_str(text).ok()?;
    let rows = rows.as_array()?;
    if rows.len() > 16 {
        return None;
    }
    let mut modes = BTreeSet::new();
    for row in rows {
        let object = row.as_object()?;
        if object.len() != 2 || !object.contains_key("width") || !object.contains_key("height") {
            return None;
        }
        if row["width"].is_null() && row["height"].is_null() {
            continue;
        }
        let (width, height) = (row["width"].as_u64()?, row["height"].as_u64()?);
        if width == 0 && height == 0 {
            continue;
        }
        let mode = format!("{width}x{height}");
        if !supported_resolution(&mode) {
            return None;
        }
        modes.insert(mode);
    }
    (modes.len() == 1).then(|| modes.into_iter().next().unwrap())
}

/// Read-only query of the signed-in user's applied display DPI (the value
/// Windows writes when the display scale changes). Nothing is written.
pub(crate) const SCALE_QUERY: &str = "(Get-ItemProperty -Path 'HKCU:\\Control Panel\\Desktop\\WindowMetrics' -Name AppliedDPI -ErrorAction Stop).AppliedDPI";
/// Windows display scale (percent) for an applied DPI: only the scales the
/// guest offers (100..=300 in steps of 25) are accepted.
pub(crate) fn scale_percent(text: &str) -> Option<u32> {
    let dpi: u32 = text.trim().parse().ok()?;
    let percent = dpi.checked_mul(100)? / 96;
    ((dpi * 100).is_multiple_of(96) && (100..=300).contains(&percent) && percent.is_multiple_of(25))
        .then_some(percent)
}
/// Host display scale hint for `desktop` auto: read-only, recorded, and
/// absent (guest integer heuristic) when unknown or not a Windows scale.
pub(super) fn resolve_scale(run: &mut Run) -> Result<Option<u32>, String> {
    let detected = if cfg!(windows) {
        let result = run.bounded(
            "powershell.exe",
            &["-NoProfile", "-NonInteractive", "-Command", SCALE_QUERY],
            Duration::from_secs(10),
            &[],
            false,
        )?;
        result
            .success
            .then(|| scale_percent(&result.stdout))
            .flatten()
    } else {
        None
    };
    run.check("desktop-native-scale-hint", json!({"requested":"auto","hint_percent":detected,
        "source":"Read-only HKCU WindowMetrics AppliedDPI; host settings are never changed",
        "guest_effect":"fw_cfg opt/door/scale: the guest's automatic scale unless the user's stored ScalePercent overrides it",
        "fallback":if detected.is_some(){Value::Null}else{json!("No applied Windows scale; guest integer heuristic")}}));
    Ok(detected)
}
pub(super) fn resolve(run: &mut Run) -> Result<String, String> {
    let detected = if cfg!(windows) {
        let result = run.bounded(
            "powershell.exe",
            &["-NoProfile", "-NonInteractive", "-Command", MODE_QUERY],
            Duration::from_secs(10),
            &[],
            false,
        )?;
        result
            .success
            .then(|| single_mode(result.stdout.trim()))
            .flatten()
    } else {
        None
    };
    let mode = detected.clone().unwrap_or_else(|| "1280x720".into());
    run.check("desktop-native-mode-selection", json!({"requested":"auto","selected":mode,
        "detected_single_supported_adapter_mode":detected.is_some(),
        "fallback":if detected.is_some(){Value::Null}else{json!("No unique supported adapter mode; conservative1280x720")},
        "source":"Read-only Win32_VideoController current mode; does not identify a primary display among distinct active modes",
        "guest_scale":"Guest automatic scale: host scale hint when offered (desktop-native-scale-hint), else the integer density policy",
        "visible_native_pixel_quality":"unqualified"}));
    Ok(mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn applied_dpi_maps_only_to_windows_display_scales() {
        let cases = [
            ("96", 100),
            ("120", 125),
            ("144\r\n", 150),
            ("168", 175),
            ("192", 200),
            ("216", 225),
            ("240", 250),
            ("288", 300),
        ];
        for (text, percent) in cases {
            assert_eq!(scale_percent(text), Some(percent), "{text:?}");
        }
        for bad in [
            "",
            "abc",
            "0",
            "72",
            "100",
            "110",
            "336",
            "-96",
            "4294967295",
            "144.0",
        ] {
            assert_eq!(scale_percent(bad), None, "{bad:?}");
        }
    }
    #[test]
    fn unique_current_mode_is_selected_without_guessing_between_monitors() {
        assert_eq!(
            single_mode(r#"[{"width":2560,"height":1600}]"#).as_deref(),
            Some("2560x1600")
        );
        assert_eq!(single_mode(r#"[{"width":3840,"height":2160},{"width":3840,"height":2160},{"width":0,"height":0}]"#).as_deref(), Some("3840x2160"));
        assert_eq!(
            single_mode(r#"[{"width":2560,"height":1600},{"width":null,"height":null}]"#)
                .as_deref(),
            Some("2560x1600")
        );
        for bad in [
            "[]",
            "{}",
            "not-json",
            r#"[{"width":2560,"height":1600},{"width":1920,"height":1080}]"#,
            r#"[{"width":999999999,"height":1600}]"#,
            r#"[{"width":2560,"height":null}]"#,
            r#"[{"width":"2560","height":1600}]"#,
            r#"[{"width":2560.5,"height":1600}]"#,
            r#"[{"width":0,"height":1600}]"#,
            r#"[{"width":2560,"height":1600,"extra":1}]"#,
        ] {
            assert!(single_mode(bad).is_none(), "{bad}");
        }
    }
}
