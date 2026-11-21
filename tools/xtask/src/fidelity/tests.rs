use super::*;

const DARK: [u8; 3] = [0x20, 0x20, 0x20];
const BAR: [u8; 3] = [0x1c, 0x24, 0x23];
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];

fn rect(x0: usize, y0: usize, x1: usize, y1: usize) -> Rect {
    Rect { x0, y0, x1, y1 }
}
fn repository_profile() -> (std::path::PathBuf, Value) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let profile = crate::image::read_json(&root.join(PROFILE)).unwrap();
    (root, profile)
}

#[test]
fn ciede2000_matches_published_reference_pairs() {
    // Sharma, Wu and Dalal (2005), Table 1 pairs 1, 7, 17 and 25.
    for (a, b, expected) in [
        ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
        ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
        ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
        (
            [60.2574, -34.0099, 36.2677],
            [60.4626, -34.1751, 39.4387],
            1.2644,
        ),
    ] {
        let got = delta_e2000_lab(a, b);
        assert!(
            (got - expected).abs() < 1e-3,
            "{a:?} {b:?}: {got} vs {expected}"
        );
    }
    assert_eq!(delta_e2000(DARK, DARK), 0.0);
    assert!(delta_e2000([0x2a; 3], [0x2c; 3]) < FROZEN.colour_delta_e);
    assert!(delta_e2000([0x2a; 3], [0xf3; 3]) > 50.0);
    assert_eq!(parse_hex("#1C2423"), Some(BAR));
    assert_eq!(hex(BAR), "#1C2423");
    assert_eq!(parse_hex("1C2423"), None);
}

#[test]
fn ppm_and_png_captures_decode_to_the_same_pixels() {
    let dir = crate::test_support::Workspace::new();
    let mut image = Image::filled(4, 3, DARK);
    image.fill_rect(rect(1, 1, 3, 2), WHITE);
    let ppm = dir.root.join("a.ppm");
    let mut bytes = b"P6\n4 3\n255\n".to_vec();
    bytes.extend_from_slice(&image.rgb);
    std::fs::write(&ppm, bytes).unwrap();
    assert_eq!(Image::read(&ppm).unwrap(), image);
    let png = dir.root.join("a.png");
    vis01_png(&image, &png);
    assert_eq!(Image::read(&png).unwrap(), image);
    std::fs::write(&ppm, b"P6\n4 3\n255\n\x00").unwrap();
    assert!(Image::read(&ppm).is_err());
}
fn vis01_png(image: &Image, path: &Path) {
    let mut png = resvg::tiny_skia::Pixmap::new(image.width as u32, image.height as u32).unwrap();
    for (rgb, rgba) in image
        .rgb
        .chunks_exact(3)
        .zip(png.data_mut().chunks_exact_mut(4))
    {
        rgba[..3].copy_from_slice(rgb);
        rgba[3] = 255;
    }
    png.save_png(path).unwrap();
}

/// A synthetic 150 % desktop: wallpaper, a 72 px taskbar with a 1 px top edge,
/// a 30x30 glyph and a two-line "clock" block.
fn synthetic_desktop() -> Image {
    let mut image = Image::filled(1200, 600, [0x50, 0x30, 0x90]);
    image.fill_rect(rect(0, 528, 1200, 600), BAR);
    image.fill_rect(rect(0, 528, 1200, 529), [0x32, 0x39, 0x4a]);
    image.fill_rect(rect(500, 549, 530, 579), WHITE);
    image.fill_rect(rect(1080, 548, 1150, 562), WHITE);
    image.fill_rect(rect(1080, 566, 1160, 580), WHITE);
    image
}

#[test]
fn measurements_find_bands_ink_and_dominant_colours() {
    let t = FROZEN;
    let desktop = synthetic_desktop();
    // The band stops at the lighter 1 px top edge: 71 of 72 rows.
    assert_eq!(band_height(&desktop, 60, 240, 144, &t), 71);
    let glyph = first_ink_box(&desktop, rect(0, 528, 1200, 600), 360, 66, &t).unwrap();
    assert_eq!(glyph, rect(500, 549, 530, 579));
    // A region holding only the bar has no ink.
    assert_eq!(ink_box(&desktop, rect(100, 535, 300, 595), &t), None);
    let clock = ink_box(&desktop, rect(1000, 530, 1190, 598), &t).unwrap();
    assert_eq!(clock, rect(1080, 548, 1160, 580));
    let (colour, share) = dominant_colour(&desktop, rect(0, 0, 1200, 600));
    assert_eq!(colour, [0x50, 0x30, 0x90]);
    assert!(share > 0.85);
    // Surface extent: a 600 px panel interrupted by an icon on one row.
    let mut start = Image::filled(1200, 600, [0x10; 3]);
    start.fill_rect(rect(300, 100, 900, 500), [0x2a; 3]);
    start.fill_rect(rect(580, 290, 620, 300), WHITE);
    assert_eq!(
        run_width(&start, rect(500, 250, 700, 350), [0x2a; 3], &t),
        600
    );
}

#[test]
fn probes_report_logical_units_at_the_capture_scale() {
    let t = FROZEN;
    let desktop = synthetic_desktop();
    let probe = |kind: &str, params: Value| json!({"kind":kind,"params":params});
    let band = measure(
        &probe(
            "band_height",
            json!({"columns_fraction":[0.05,0.2],"max_logical":96}),
        ),
        &desktop,
        150,
        &t,
    )
    .unwrap();
    assert_eq!(band["height"], 47.33);
    let glyph = measure(
        &probe(
            "first_ink_box",
            json!({"band_logical":48,"start_fraction":0.3,"cluster_logical":44}),
        ),
        &desktop,
        150,
        &t,
    )
    .unwrap();
    assert_eq!(
        (glyph["width"].as_f64(), glyph["height"].as_f64()),
        (Some(20.0), Some(20.0))
    );
    assert_eq!(glyph["centre_dy"], 0.0);
    let clock = measure(
        &probe(
            "ink_box",
            json!({"region":{"anchor":"bottom_right","x":-150,"y":-48,"w":142,"h":48}}),
        ),
        &desktop,
        150,
        &t,
    )
    .unwrap();
    assert_eq!(clock["height"], 21.33);
    let colour = measure(
        &probe(
            "dominant_colour",
            json!({"region":{"anchor":"center","x":-50,"y":-50,"w":100,"h":100}}),
        ),
        &desktop,
        150,
        &t,
    )
    .unwrap();
    assert_eq!(colour["rgb"], "#503090");
    // The same physical feature at 200 % is smaller in logical units.
    assert_eq!(
        measure(
            &probe(
                "band_height",
                json!({"columns_fraction":[0.05,0.2],"max_logical":96})
            ),
            &desktop,
            200,
            &t
        )
        .unwrap()["height"],
        35.5
    );
    assert!(measure(&probe("mystery", json!({})), &desktop, 150, &t).is_err());
    assert!(
        measure(
            &probe(
                "ink_box",
                json!({"region":{"anchor":"top_left","x":5000,"y":0,"w":10,"h":10}})
            ),
            &desktop,
            150,
            &t
        )
        .is_err()
    );
}

#[test]
fn settle_time_is_the_first_frame_after_which_nothing_changes() {
    let mut frames = Vec::new();
    for (i, x) in [0usize, 40, 80, 100, 100, 100].into_iter().enumerate() {
        let mut image = Image::filled(200, 100, DARK);
        image.fill_rect(rect(x, 10, x + 20, 30), WHITE);
        frames.push((i as f64 * 50.0, image));
    }
    let whole = |image: &Image| Ok(rect(0, 0, image.width, image.height));
    assert_eq!(settle_ms(&frames, &whole).unwrap(), Some(150.0));
    // Still moving at the end of the window: not settled.
    frames.truncate(3);
    assert_eq!(settle_ms(&frames, &whole).unwrap(), None);
    assert_eq!(settle_ms(&[], &whole).unwrap(), None);
}

fn synthetic_profile() -> Value {
    let region = json!({"anchor":"center","x":-50,"y":-50,"w":100,"h":100});
    json!({"schema_version":1,"record_kind":"windows_reference","id":"synthetic",
    "thresholds":FROZEN.to_json(),
    "surfaces":[{"id":"lock-logon","claimed":false,"exclusion":"secure desktop","captures":[]}],
    "probes":[
        {"id":"bar.height","dimension":"geometry","surface":"desktop","kind":"band_height","status":"frozen",
         "params":{"columns_fraction":[0.05,0.2],"max_logical":96},"compare":["height"],"expected":{"height":48}},
        {"id":"wall.colour","dimension":"colour","surface":"desktop","kind":"dominant_colour","status":"frozen",
         "params":{"region":region},"compare":["rgb"],"expected":{"rgb":"#503090"}},
        {"id":"clock","dimension":"typography","surface":"desktop","kind":"ink_box","status":"frozen",
         "params":{"region":{"anchor":"bottom_right","x":-150,"y":-48,"w":142,"h":48}},"compare":["height"],"expected":{"height":21}},
        {"id":"glyph","dimension":"iconography","surface":"desktop","kind":"first_ink_box","status":"approved_deviation",
         "params":{"band_logical":48,"start_fraction":0.3,"cluster_logical":44},"compare":["width"],"expected":{"width":16},"deviation":"door-mark"},
        {"id":"bar.colour","dimension":"colour","surface":"desktop","kind":"dominant_colour","status":"uncontrolled",
         "params":{"region":{"anchor":"bottom_left","x":16,"y":-40,"w":160,"h":32}},"compare":["rgb"],"expected":{"rgb":"#FFFFFF"}},
        {"id":"open","dimension":"motion","surface":"start-open","kind":"settle_ms","status":"frozen",
         "params":{"region":region},"compare":["duration_ms"],"expected":{"duration_ms":120}}
    ]})
}
fn desktop_capture(image: Image) -> Capture {
    Capture {
        surface: "desktop".into(),
        image,
        scale_percent: 150,
        path: "reports/runs/x/vis01-desktop.ppm".into(),
        sha256: "0".repeat(64),
    }
}
fn opening(settle_at: usize) -> Sequence {
    let frames = (0..6)
        .map(|i| {
            let mut image = Image::filled(1200, 600, DARK);
            let x = 500 + 40 * i.min(settle_at);
            image.fill_rect(rect(x, 250, x + 60, 350), WHITE);
            (i as f64 * 40.0, image)
        })
        .collect();
    Sequence {
        surface: "start-open".into(),
        frames,
        scale_percent: 150,
        sample_interval_ms: 40.0,
    }
}

#[test]
fn comparison_reports_every_probe_and_fails_closed() {
    let root = Path::new(".");
    let profile = synthetic_profile();
    let report = compare(
        root,
        &profile,
        &[desktop_capture(synthetic_desktop())],
        &[opening(3)],
    )
    .unwrap();
    let outcome = |id: &str| {
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .unwrap()["outcome"]
            .clone()
    };
    assert_eq!(outcome("bar.height"), "passed"); // 47.33 vs 48 within 2
    assert_eq!(outcome("wall.colour"), "passed");
    assert_eq!(outcome("clock"), "passed"); // 21.33 vs 21 within 1.5
    assert_eq!(outcome("glyph"), "exception"); // 20 vs 16, approved deviation
    assert_eq!(outcome("bar.colour"), "reported");
    assert_eq!(outcome("open"), "passed"); // settles at 120 ms
    assert_eq!(report["outcome"], "passed");
    assert_eq!(report["exceptions"][0]["deviation"], "door-mark");
    assert_eq!(report["excluded_surfaces"][0]["surface"], "lock-logon");
    let short = summary(&report, "abc");
    assert_eq!(short["dimensions"]["motion"], "passed");

    // A localized failure fails its dimension and the suite, by name.
    let mut moved = synthetic_desktop();
    moved.fill_rect(rect(0, 500, 1200, 529), BAR);
    let report = compare(root, &profile, &[desktop_capture(moved)], &[opening(3)]).unwrap();
    assert_eq!(report["dimensions"]["geometry"]["outcome"], "failed");
    assert_eq!(report["deviations"], json!(["bar.height"]));
    assert_eq!(report["outcome"], "failed");

    // Missing guest captures and unresolved references never pass.
    let report = compare(root, &profile, &[], &[]).unwrap();
    assert_eq!(report["dimensions"]["colour"]["outcome"], "failed");
    let mut unresolved = profile.clone();
    unresolved["probes"][5]["status"] = json!("not_captured");
    let report = compare(
        root,
        &unresolved,
        &[desktop_capture(synthetic_desktop())],
        &[opening(3)],
    )
    .unwrap();
    assert_eq!(report["dimensions"]["motion"]["outcome"], "unresolved");
    assert_eq!(report["outcome"], "failed");
    // A sequence still moving at the end fails its motion probe.
    let report = compare(
        root,
        &profile,
        &[desktop_capture(synthetic_desktop())],
        &[opening(9)],
    )
    .unwrap();
    assert_eq!(report["dimensions"]["motion"]["outcome"], "failed");
    // A dimension with no evaluated probe is not a pass.
    let mut no_motion = profile.clone();
    no_motion["probes"].as_array_mut().unwrap().remove(5);
    let report = compare(
        root,
        &no_motion,
        &[desktop_capture(synthetic_desktop())],
        &[],
    )
    .unwrap();
    assert_eq!(report["dimensions"]["motion"]["outcome"], "not_evaluated");
    assert_eq!(report["outcome"], "failed");
}

#[test]
fn thresholds_and_reference_bytes_cannot_change_after_freezing() {
    let root = crate::test_support::Workspace::new();
    let mut profile = synthetic_profile();
    assert!(thresholds_match(&profile["thresholds"]));
    profile["thresholds"]["colour_delta_e2000"] = json!(9.0);
    assert!(
        compare(&root.root, &profile, &[], &[])
            .unwrap_err()
            .contains("thresholds")
    );
    let mut profile = synthetic_profile();
    profile["thresholds"]["geometry_logical"] = json!(2); // integer spelling of 2.0
    assert!(thresholds_match(&profile["thresholds"]));
    std::fs::create_dir_all(root.root.join("reports/artifacts/reference-26h2")).unwrap();
    std::fs::write(
        root.root.join("reports/artifacts/reference-26h2/x.png"),
        b"bytes",
    )
    .unwrap();
    profile["surfaces"][0]["captures"] =
        json!([{"path":"reports/artifacts/reference-26h2/x.png","sha256":"0".repeat(64)}]);
    assert!(
        compare(&root.root, &profile, &[], &[])
            .unwrap_err()
            .contains("changed")
    );
    profile["surfaces"][0]["captures"][0]["sha256"] = json!(
        crate::evidence::file_hash(&root.root.join("reports/artifacts/reference-26h2/x.png"))
            .unwrap()
    );
    assert!(compare(&root.root, &profile, &[], &[]).is_ok());
    profile["record_kind"] = json!("other");
    assert!(compare(&root.root, &profile, &[], &[]).is_err());
}

#[test]
fn repository_profile_is_frozen_complete_and_privacy_preserving() {
    let (root, profile) = repository_profile();
    assert!(thresholds_match(&profile["thresholds"]));
    assert_eq!(profile["record_kind"], "windows_reference");
    assert_eq!(profile["edition"], "Windows 11 Pro");
    assert_eq!(profile["display_version"], "26H2");
    assert_eq!(profile["os_build"], "26300.9711");
    assert_eq!(profile["displays"][0]["scale_percent"], 150);
    // The section 112 inventory: 15 surface families, each claimed with
    // captures or excluded with a reason.
    let families = [
        "Desktop",
        "Taskbar",
        "Start/launcher",
        "Search",
        "System tray",
        "Quick settings",
        "Notification center/calendar",
        "Task switching",
        "Snap and window management",
        "File manager",
        "File operations",
        "Settings",
        "Common dialogs",
        "Lock/logon",
        "Installation/recovery",
    ];
    let surfaces = profile["surfaces"].as_array().unwrap();
    assert_eq!(surfaces.len(), families.len());
    for (surface, family) in surfaces.iter().zip(families) {
        assert_eq!(surface["family"], family);
        if surface["claimed"] == true {
            assert!(
                !surface["captures"].as_array().unwrap().is_empty(),
                "{family}"
            );
        } else {
            assert!(
                surface["exclusion"].as_str().is_some_and(|r| r.len() > 20),
                "{family}"
            );
        }
    }
    // Every dimension has at least one probe; frozen probes carry values.
    let probes = profile["probes"].as_array().unwrap();
    for d in Dimension::ALL {
        assert!(
            probes.iter().any(|p| p["dimension"] == d.name()),
            "{}",
            d.name()
        );
    }
    for p in probes {
        Dimension::parse(p["dimension"].as_str().unwrap()).unwrap();
        if p["status"] != "not_captured" {
            for field in p["compare"].as_array().unwrap() {
                assert!(
                    !p["expected"][field.as_str().unwrap()].is_null(),
                    "{}",
                    p["id"]
                );
            }
        }
    }
    // Personal strings from the captures never enter the profile. They are
    // kept as (length, FNV-1a 64 of the lowercase text), so this public test
    // does not spell out the owner's details.
    const PRIVATE: [(usize, u64); 6] = [
        (6, 0x3ec4b34002d1ee5f),
        (5, 0x3fd66346a6aae0c9),
        (5, 0xaa56c91f2f0ea3f3),
        (6, 0x520f3a326e4de9f7),
        (8, 0xecb826a3489df49a),
        (6, 0xfcf9c7de731d5647),
    ];
    let text = std::fs::read_to_string(root.join(PROFILE))
        .unwrap()
        .to_ascii_lowercase();
    for (length, hash) in PRIVATE {
        for window in text.as_bytes().windows(length) {
            let fnv = window.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
            });
            assert_ne!(fnv, hash, "a personal string entered the profile");
        }
    }
    // Reference bytes, where present, are the frozen ones.
    compare(&root, &profile, &[], &[]).unwrap();
}

#[test]
fn frozen_reference_values_reproduce_from_the_reference_captures() {
    let (root, profile) = repository_profile();
    let t = FROZEN;
    for p in profile["probes"].as_array().unwrap() {
        if p["reference"]["method"] != "harness" {
            continue;
        }
        let capture = root.join(p["reference"]["capture"].as_str().unwrap());
        if !capture.exists() {
            continue; // Private evidence absent: the frozen values stand.
        }
        let scale = p["reference"]["scale_percent"].as_u64().unwrap() as u32;
        let measured = measure(p, &Image::read(&capture).unwrap(), scale, &t).unwrap();
        for field in p["compare"].as_array().unwrap() {
            let field = field.as_str().unwrap();
            let (got, frozen) = (&measured[field], &p["expected"][field]);
            match (got.as_f64(), frozen.as_f64()) {
                (Some(a), Some(b)) => assert_eq!(a, b, "{} {field}", p["id"]),
                _ => assert_eq!(got, frozen, "{} {field}", p["id"]),
            }
        }
    }
}

/// `cargo test -p door-xtask measure_reference_probes -- --ignored --nocapture`
/// prints the reference measurements used to freeze the profile.
#[test]
#[ignore]
fn measure_reference_probes() {
    let (root, profile) = repository_profile();
    let mut out = serde_json::Map::new();
    for p in profile["probes"].as_array().unwrap() {
        if p["reference"]["method"] != "harness" {
            continue;
        }
        let capture = root.join(p["reference"]["capture"].as_str().unwrap());
        let scale = p["reference"]["scale_percent"].as_u64().unwrap() as u32;
        let measured = measure(p, &Image::read(&capture).unwrap(), scale, &FROZEN).unwrap();
        out.insert(p["id"].as_str().unwrap().into(), measured);
    }
    println!("REFERENCE_MEASUREMENTS {}", Value::Object(out));
}

#[test]
fn animation_time_runs_from_the_first_change_to_the_settled_frame() {
    let whole = |_: &Image| Ok(rect(0, 0, 4, 4));
    let frame = |c: u8| Image::filled(4, 4, [c, c, c]);
    // Unchanged until 40 ms, moving until 150 ms, then still.
    let frames = vec![
        (0.0, frame(0)),
        (20.0, frame(0)),
        (40.0, frame(10)),
        (100.0, frame(20)),
        (150.0, frame(30)),
        (200.0, frame(30)),
    ];
    assert_eq!(settle_ms(&frames, &whole).unwrap(), Some(150.0));
    assert_eq!(animation_ms(&frames, &whole).unwrap(), Some(110.0));
    // Never changing, or still changing at the end: no duration.
    assert_eq!(
        animation_ms(&[(0.0, frame(0)), (9.0, frame(0))], &whole).unwrap(),
        None
    );
    assert_eq!(animation_ms(&frames[..5], &whole).unwrap(), None);
}

#[test]
fn frozen_motion_reproduces_from_the_reference_sequence() {
    let (root, profile) = repository_profile();
    let mut checked = 0;
    for p in profile["probes"].as_array().unwrap() {
        if p["reference"]["method"] != "sequence" {
            continue;
        }
        let sequence = root.join(p["reference"]["sequence"].as_str().unwrap());
        if !sequence.exists() {
            continue; // Private evidence absent: the frozen value stands.
        }
        assert_eq!(p["kind"], "animation_ms", "{}", p["id"]);
        assert_eq!(
            reference_animation_ms(&root, p).unwrap(),
            p["expected"]["duration_ms"].as_f64().unwrap(),
            "{}",
            p["id"]
        );
        checked += 1;
    }
    assert!(checked > 0 || !root.join("reports/artifacts/reference-26h2").exists());
}

/// `cargo test -p door-xtask measure_reference_motion -- --ignored --nocapture`
/// prints each run's animation time from a reference sequence.
#[test]
#[ignore]
fn measure_reference_motion() {
    let (root, _) = repository_profile();
    let sequence = "reports/artifacts/reference-26h2/motion/start-open/sequence.json";
    let region =
        Region::parse(&json!({"anchor":"center","x":-200,"y":-200,"w":400,"h":400})).unwrap();
    for frames in reference_runs(&root, sequence).unwrap() {
        let at = |image: &Image| region.physical(image, 1.5);
        println!(
            "REFERENCE_MOTION settle={:?} animation={:?}",
            settle_ms(&frames, &at).unwrap(),
            animation_ms(&frames, &at).unwrap()
        );
    }
}
