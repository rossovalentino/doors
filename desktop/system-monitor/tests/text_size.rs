//! Accessibility > Text size and contrast themes on the actual Task Manager
//! layout. The applied text size is process-global (`palette::text_percent`),
//! so every test here holds `LOCK` and restores 100%.
use std::sync::Mutex;

use door_abi::telemetry::{Process, Snapshot};
use door_app_ui::{Application, ImageCache, View};
use door_graphics_text::{FontAtlas, TextStyle};
use door_monitor::{Monitor, PAGE_NAMES};
use door_service_protocol::desktop::ContrastTheme;
use door_workspace::palette::set_text_percent;
use door_workspace::semantics::Semantics;

static LOCK: Mutex<()> = Mutex::new(());

/// Runs `f` at a text size, then restores 100% even if `f` panics.
fn at_text<T>(text: u32, f: impl FnOnce() -> T) -> T {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            set_text_percent(100);
        }
    }
    let _restore = Restore;
    set_text_percent(text);
    f()
}

fn session(n: u64) -> Snapshot {
    let mut s = Snapshot {
        ticks: n * 100,
        cpus: 1,
        total_samples: n * 100,
        busy_samples: n * 37,
        managed_pages: 512 * 1024,
        allocated_pages: 150_000,
        ..Snapshot::default()
    };
    let catalog: [(u32, u32); 6] = [(1, 1), (2, 2), (3, 3), (12, 1001), (13, 1001), (14, 1001)];
    s.count = catalog.len();
    for (i, (manifest, uid)) in catalog.into_iter().enumerate() {
        s.processes[i] = Process {
            pid: 40 + i as u64,
            instance: 9 + i as u64,
            manifest,
            uid,
            mode: 0,
            state: 3,
            threads: 1 + i as u32 % 3,
            samples: n * (i as u64 % 4 + 1),
            private_bytes: (8 + i as u64 * 6) * 1024 * 1024,
        };
    }
    s
}

fn app(fonts: &FontAtlas) -> Monitor {
    let mut app = Monitor::new();
    for n in 1..=4 {
        app.ingest(session(n));
    }
    app.configure_fonts(1100, 720, fonts);
    app
}

/// (target, bounds, name) of one semantics node that answers input.
type Node = (u32, (i32, i32, u32, u32), String);

/// Every semantics node that answers input.
fn nodes(app: &Monitor, fonts: &FontAtlas) -> Vec<Node> {
    let mut s = Semantics::new(1100, 720);
    app.semantics(&mut s, fonts);
    let built = s.finish(1, 1, "Task Manager");
    built
        .targets
        .iter()
        .filter_map(|(id, target)| {
            let n = built.tree.get(*id)?;
            let (x, y, w, h) = n.bounds.logical();
            Some((
                *target,
                (x, y, w as u32, h as u32),
                String::from(n.name.as_str()),
            ))
        })
        .collect()
}

/// Every node's bounds are its hit-test region: all four inner corners
/// answer the node's target.
fn assert_semantics_match_hits(app: &Monitor, nodes: &[Node], what: &str) {
    for (target, (x, y, w, h), name) in nodes {
        if *w < 2 || *h < 2 {
            continue;
        }
        let (r, b) = (x + *w as i32 - 1, y + *h as i32 - 1);
        for (px, py) in [(*x, *y), (r, *y), (*x, b), (r, b)] {
            assert_eq!(
                app.target(px, py),
                Some(*target),
                "{what}: {name:?} ({target}) at {px},{py} of {:?}",
                (x, y, w, h)
            );
        }
    }
}

fn find(nodes: &[Node], range: core::ops::Range<u32>) -> Vec<&Node> {
    nodes.iter().filter(|n| range.contains(&n.0)).collect()
}

fn width(fonts: &FontAtlas, text: &str, style: TextStyle) -> u32 {
    fonts.measure(text, style).map_or(0, |m| m.advance_width)
}

/// Rows, headers, navigation items, commands and tiles grow; text fits.
fn check(display: u32, text: u32) {
    let base_fonts = FontAtlas::windows_reference_text(display, 100).unwrap();
    let base = nodes(&app(&base_fonts), &base_fonts);
    let fonts = FontAtlas::windows_reference_text(display, text).unwrap();
    assert_eq!(fonts.text_percent(), text);
    at_text(text, || {
        let app = app(&fonts);
        let all = nodes(&app, &fonts);
        let what = format!("display {display}% text {text}%");
        assert_semantics_match_hits(&app, &all, &what);
        let line14 = fonts.line_height(TextStyle::Regular14);
        let line12 = fonts.line_height(TextStyle::Regular12);
        // Process rows.
        let rows = find(&all, 100..400);
        assert!(!rows.is_empty(), "{what}: no process rows");
        // Each row grows past the same row at 100% (group rows are taller
        // than process rows, so rows compare with their own counterpart).
        let base_rows = find(&base, 100..400);
        for (id, (_, _, _, h), name) in rows.iter().take(rows.len() - 1) {
            let base_row = base_rows.iter().find(|b| b.0 == *id).map_or(0, |b| b.1.3);
            assert!(*h > base_row, "{what}: row {name} {h} <= {base_row}");
            assert!(*h >= line14 + 8, "{what}: row {name} {h} < line {line14}");
        }
        // Column headers hold their 12 px label under the total line.
        for (_, (_, _, w, h), name) in find(&all, 50..70) {
            assert!(
                *h >= line12 + fonts.line_height(TextStyle::Regular16),
                "{what}: header {name} {h}"
            );
            assert!(
                *w >= width(&fonts, name, TextStyle::Regular12) + 20,
                "{what}: header {name} {w}"
            );
        }
        // Navigation items: taller, and the label fits after its icon.
        let nav = find(&all, 10..18);
        assert_eq!(nav.len(), 8, "{what}");
        let base_nav = find(&base, 10..18)[0].1.3;
        for (_, (_, _, w, h), name) in &nav {
            assert!(*h > base_nav && *h >= line14 + 8, "{what}: nav {name} {h}");
            assert!(
                44 + width(&fonts, name, TextStyle::Regular14) + 8 <= *w,
                "{what}: nav {name} {w}"
            );
        }
        // Header commands: tall enough, and either wide enough for their
        // labels or collapsed to 40 px icon buttons (the label is a tooltip).
        for (_, (_, _, w, h), name) in find(&all, 20..40) {
            assert!(*h >= line14 + 8, "{what}: command {name} {h}");
            assert!(
                *w == 40 || *w >= 40 + width(&fonts, name, TextStyle::Regular14),
                "{what}: command {name} {w}"
            );
        }
        // The page title fits its header band above the table.
        let header = fonts.line_height(TextStyle::Semibold20);
        let first_row = rows[0].1.1;
        assert!(
            56 + header as i32 <= first_row,
            "{what}: title {header} reaches the table at {first_row}"
        );
    });
}

#[test]
fn rows_navigation_and_commands_grow_and_hold_their_text_at_225() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    check(100, 225);
}

#[test]
fn display_150_times_text_150_grows_with_semantics_matching_hits() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    check(150, 150);
}

#[test]
fn performance_settings_and_menu_layouts_grow_with_matching_semantics() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fonts = FontAtlas::windows_reference_text(100, 225).unwrap();
    at_text(225, || {
        let mut app = app(&fonts);
        let line14 = fonts.line_height(TextStyle::Regular14);
        // Performance tiles (nav id 11 opens page 1).
        assert!(click_target(&mut app, &fonts, 11));
        let all = nodes(&app, &fonts);
        assert_semantics_match_hits(&app, &all, "performance");
        let tiles = find(&all, 40..43);
        assert_eq!(tiles.len(), 3);
        for (_, (_, _, _, h), name) in tiles {
            assert!(
                *h >= fonts.line_height(TextStyle::Regular16)
                    + fonts.line_height(TextStyle::Regular12)
                    + 16,
                "tile {name} {h}"
            );
        }
        // Settings: combo boxes and check boxes hold their labels.
        assert!(click_target(&mut app, &fonts, 17));
        let all = nodes(&app, &fonts);
        assert_semantics_match_hits(&app, &all, "settings");
        for (target, (_, _, w, h), name) in find(&all, 700..704) {
            assert!(*h >= line14 + 8, "setting {name} {h}");
            if *target >= 702 {
                assert!(
                    *w >= 30 + width(&fonts, name, TextStyle::Regular14),
                    "check {name} {w}"
                );
            }
        }
        // The Processes View menu: taller items in a card wide enough.
        assert!(click_target(&mut app, &fonts, 10));
        assert!(click_target(&mut app, &fonts, 23));
        let all = nodes(&app, &fonts);
        let items = find(&all, 500..520);
        assert!(!items.is_empty(), "View menu did not open");
        for (_, (_, _, w, h), name) in items {
            assert!(*h >= line14 + 8, "menu {name} {h}");
            assert!(
                *w >= 44 + width(&fonts, name, TextStyle::Regular14),
                "menu {name} {w}"
            );
        }
    });
}

/// Press the semantics node with `id` through its own hit test.
fn click_target(app: &mut Monitor, fonts: &FontAtlas, id: u32) -> bool {
    let Some((_, (x, y, w, h), _)) = nodes(app, fonts).into_iter().find(|n| n.0 == id) else {
        return false;
    };
    let Some(target) = app.target(x + w as i32 / 2, y + h as i32 / 2) else {
        return false;
    };
    app.press(target)
}

#[test]
fn contrast_themes_paint_window_highlight_and_no_heat_map() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let app = app(&fonts);
    let nav = find(&nodes(&app, &fonts), 10..11)[0].1;
    let rows = nodes(&app, &fonts);
    let row = find(&rows, 100..400)[1].1;
    for theme in ContrastTheme::ALL {
        let s = theme.colors();
        let bgra = |c: [u8; 3]| [c[2], c[1], c[0], 255];
        let cache = ImageCache::default();
        let mut pixels = vec![0; 1100 * 720 * 4];
        let mut v = View::new_scaled(&mut pixels, 1100, 720, &fonts, 1).with_images(&cache);
        v.palette.dark = false;
        v.palette.contrast = Some(theme);
        app.draw(&mut v);
        let at = |x: i32, y: i32| {
            let i = (y as usize * 1100 + x as usize) * 4;
            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
        };
        // The selected destination is a Highlight plate.
        assert_eq!(at(nav.0 + 6, nav.1 + 3), bgra(s.highlight), "{theme:?} nav");
        // Window behind the table; a busy CPU cell keeps it (no heat map).
        let (x, y, w, _) = row;
        assert_eq!(at(x + 2, y + 2), bgra(s.window), "{theme:?} row");
        assert_eq!(
            at(x + w as i32 - 3, y + 2),
            bgra(s.window),
            "{theme:?} memory cell"
        );
        // Page title ink is WindowText.
        assert_eq!(PAGE_NAMES[0], "Processes");
        let left = nav.0 + nav.2 as i32 + 4 + 24;
        let ink = (56..92)
            .flat_map(|py| (left..left + 120).map(move |px| (px, py)))
            .any(|(px, py)| at(px, py) == bgra(s.window_text));
        assert!(ink, "{theme:?} title ink");
    }
}
