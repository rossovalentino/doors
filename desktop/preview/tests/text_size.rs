//! Accessibility > Text size and contrast themes on the actual Preview
//! layout. The applied text size is process-global (`palette::text_percent`),
//! so every test here holds `LOCK` and restores 100%.
use std::sync::Mutex;

use door_app_ui::{Application, ImageCache, View};
use door_graphics_text::{FontAtlas, TextStyle};
use door_preview::Preview;
use door_service_protocol::desktop::ContrastTheme;
use door_workspace::palette::set_text_percent;
use door_workspace::semantics::{Role, Semantics};

static LOCK: Mutex<()> = Mutex::new(());
const IMAGE: &[u8] = b"P3 2 2 255 255 0 0 0 255 0 0 0 255 255 255 255";

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

struct Node {
    target: Option<u32>,
    role: Role,
    name: String,
    r: (i32, i32, u32, u32),
}

fn app(width: u32, height: u32) -> Preview {
    let mut app = Preview::new(1001);
    app.configure(width, height);
    assert!(app.open_image_for_review("/users/1001/photo.ppm", IMAGE));
    app
}

fn nodes(app: &Preview, width: u32, height: u32, fonts: &FontAtlas) -> Vec<Node> {
    let mut s = Semantics::new(width, height);
    app.semantics(&mut s, fonts);
    let built = s.finish(1, 1, "Preview");
    built
        .tree
        .nodes()
        .map(|n| {
            let (x, y, w, h) = n.bounds.logical();
            Node {
                target: built
                    .targets
                    .iter()
                    .find(|(id, _)| *id == n.id)
                    .map(|(_, t)| *t),
                role: n.role,
                name: String::from(n.name.as_str()),
                r: (x, y, w as u32, h as u32),
            }
        })
        .collect()
}

fn width(fonts: &FontAtlas, text: &str, style: TextStyle) -> u32 {
    fonts.measure(text, style).map_or(0, |m| m.advance_width)
}

/// Semantics bounds are the hit-test regions: the node's corners (centre for
/// caption tabs, whose close buttons sit inside them) answer its target.
fn assert_semantics_match_hits(app: &Preview, nodes: &[Node], what: &str) {
    for n in nodes {
        let Some(target) = n.target else { continue };
        let (x, y, w, h) = n.r;
        if w < 2 || h < 2 {
            continue;
        }
        let points = if (100..400).contains(&target) {
            vec![(x + w as i32 / 2, y + h as i32 / 2)]
        } else {
            let (r, b) = (x + w as i32 - 1, y + h as i32 - 1);
            vec![(x, y), (r, y), (x, b), (r, b)]
        };
        for (px, py) in points {
            assert_eq!(
                app.target(px, py),
                Some(target),
                "{what}: {:?} ({target}) at {px},{py} of {:?}",
                n.name,
                n.r
            );
        }
    }
}

fn targets(nodes: &[Node], range: core::ops::Range<u32>) -> Vec<&Node> {
    nodes
        .iter()
        .filter(|n| n.target.is_some_and(|t| range.contains(&t)))
        .collect()
}

fn check(display: u32, text: u32) {
    let (w, h) = (1280, 900);
    let base_fonts = FontAtlas::windows_reference_text(display, 100).unwrap();
    let base = nodes(&app(w, h), w, h, &base_fonts);
    let base_nav = targets(&base, 400..417)[0].r.3;
    let base_row = base
        .iter()
        .find(|n| n.role == Role::Text && n.name.starts_with("Type"))
        .unwrap()
        .r
        .3;
    let fonts = FontAtlas::windows_reference_text(display, text).unwrap();
    at_text(text, || {
        let what = format!("display {display}% text {text}%");
        let line14 = fonts.line_height(TextStyle::Regular14);
        let app = app(w, h);
        let all = nodes(&app, w, h, &fonts);
        assert_semantics_match_hits(&app, &all, &what);
        // Navigation rows: taller, and each label fits after its icon.
        let nav = targets(&all, 400..417);
        assert!(nav.len() >= 4, "{what}");
        for n in nav
            .iter()
            .filter(|n| n.r.3 >= door_workspace::nav_pane::row_height())
        {
            assert!(
                n.r.3 > base_nav && n.r.3 >= line14 + 8,
                "{what}: nav {} {:?}",
                n.name,
                n.r
            );
            assert!(
                55 + width(&fonts, &n.name, TextStyle::Regular14) + 8 <= n.r.2,
                "{what}: nav {} {:?}",
                n.name,
                n.r
            );
        }
        // Toolbar zoom value box holds its label.
        let zoom = all
            .iter()
            .find(|n| n.target == Some(3) && n.r.2 > 36)
            .expect("zoom box");
        assert!(zoom.r.3 >= line14 + 8, "{what}: zoom {:?}", zoom.r);
        assert!(
            zoom.r.2 >= 24 + width(&fonts, "100%", TextStyle::Regular14),
            "{what}: zoom {:?}",
            zoom.r
        );
        // Information rows grow, stack without overlap, and end above the
        // export button; names fit their column.
        let rows: Vec<&Node> = all
            .iter()
            .filter(|n| n.role == Role::Text && n.name.contains(": "))
            .collect();
        assert!(rows.len() >= 2, "{what}: {} info rows", rows.len());
        for pair in rows.windows(2) {
            assert!(
                pair[0].r.1 + pair[0].r.3 as i32 <= pair[1].r.1,
                "{what}: rows overlap"
            );
        }
        // The pane export button (the toolbar also has a Save button, 7).
        let export: Vec<&Node> = targets(&all, 7..8)
            .into_iter()
            .filter(|n| n.r.1 > 140)
            .collect();
        for n in &rows {
            assert!(
                n.r.3 > base_row && n.r.3 >= line14 + 8,
                "{what}: row {} {:?}",
                n.name,
                n.r
            );
            let name = n.name.split(": ").next().unwrap();
            let column = door_workspace::palette::text_len(148).min(n.r.2 / 2);
            assert!(
                width(&fonts, name, TextStyle::Regular14) <= column,
                "{what}: row name {name}"
            );
            if let Some(e) = export.first() {
                assert!(
                    n.r.1 + n.r.3 as i32 <= e.r.1,
                    "{what}: row {} under export {:?}",
                    n.name,
                    e.r
                );
            }
        }
        let more = all.iter().find(|n| n.target == Some(9)).expect("Show more");
        assert!(
            more.r.3 >= line14 + 8 && more.r.2 >= width(&fonts, "Show more", TextStyle::Regular14),
            "{what}: {:?}",
            more.r
        );
    });
}

#[test]
fn navigation_toolbar_and_information_rows_grow_and_hold_text_at_225() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    check(100, 225);
}

#[test]
fn display_150_times_text_150_grows_with_semantics_matching_hits() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    check(150, 150);
}

#[test]
fn quick_actions_and_section_rows_grow_with_matching_semantics() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fonts = FontAtlas::windows_reference_text(100, 225).unwrap();
    at_text(225, || {
        let line14 = fonts.line_height(TextStyle::Regular14);
        // A tall window shows Quick actions and Convert.
        let (w, h) = (1280, 1500);
        let mut app = app(w, h);
        let all = nodes(&app, w, h, &fonts);
        assert_semantics_match_hits(&app, &all, "quick actions");
        for id in [206, 5, 6] {
            let n = all
                .iter()
                .find(|n| n.target == Some(id) && n.r.0 >= w as i32 - 388 && n.r.1 > 140)
                .unwrap_or_else(|| panic!("quick action {id}"));
            assert!(n.r.3 >= 58 + line14 + 8, "quick action {id} {:?}", n.r);
        }
        for n in all
            .iter()
            .filter(|n| matches!(n.target, Some(7 | 203)) && n.r.0 >= w as i32 - 388 && n.r.1 > 140)
        {
            assert!(n.r.3 >= line14 + 18, "convert {:?}", n.r);
            assert!(n.r.1 + n.r.3 as i32 + 24 <= h as i32);
        }
        // Library rows hold a file name above its path.
        app.show_section(3);
        let all = nodes(&app, w, h, &fonts);
        assert_semantics_match_hits(&app, &all, "library");
        let rows = targets(&all, 600..700);
        assert!(!rows.is_empty());
        let need =
            fonts.line_height(TextStyle::Regular16) + fonts.line_height(TextStyle::Regular12) + 8;
        for n in rows {
            assert!(n.r.3 >= need, "library row {:?}", n.r);
        }
        // Settings rows hold their label and value button.
        app.show_section(16);
        let all = nodes(&app, w, h, &fonts);
        assert_semantics_match_hits(&app, &all, "settings");
        let rows: Vec<&Node> = all
            .iter()
            .filter(|n| matches!(n.target, Some(202..=204)) && n.r.2 > 200)
            .collect();
        assert_eq!(rows.len(), 3);
        for n in rows {
            assert!(n.r.3 >= line14 + 2 * 14, "settings row {:?}", n.r);
        }
    });
}

#[test]
fn contrast_themes_map_preview_surfaces_and_text() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let (w, h) = (1280, 900);
    let app = app(w, h);
    let all = nodes(&app, w, h, &fonts);
    let row = all
        .iter()
        .find(|n| n.role == Role::Text && n.name.starts_with("Type"))
        .unwrap()
        .r;
    for theme in ContrastTheme::ALL {
        let s = theme.colors();
        let bgra = |c: [u8; 3]| [c[2], c[1], c[0], 255];
        let cache = ImageCache::default();
        let mut pixels = vec![0; (w * h * 4) as usize];
        let mut v = View::new_scaled(&mut pixels, w, h, &fonts, 1).with_images(&cache);
        v.palette.dark = false;
        v.palette.contrast = Some(theme);
        app.draw(&mut v);
        let at = |x: i32, y: i32| {
            let i = (y as usize * w as usize + x as usize) * 4;
            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
        };
        // Information pane background is Window; its row text WindowText/GrayText.
        assert_eq!(
            at(row.0 + row.2 as i32 - 2, row.1 + 1),
            bgra(s.window),
            "{theme:?} pane"
        );
        let ink: Vec<[u8; 4]> = (row.1..row.1 + row.3 as i32)
            .flat_map(|y| (row.0..row.0 + row.2 as i32).map(move |x| (x, y)))
            .map(|(x, y)| at(x, y))
            .collect();
        assert!(ink.contains(&bgra(s.window_text)), "{theme:?} value ink");
    }
}
