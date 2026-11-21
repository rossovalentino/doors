extern crate std;
use super::*;
use door_app_ui::fluent::Rect as R;

fn sample(ticks: u64, total: u64, busy: u64) -> Snapshot {
    Snapshot {
        ticks,
        cpus: 1,
        total_samples: total,
        busy_samples: busy,
        managed_pages: 1000,
        allocated_pages: 400,
        ..Snapshot::default()
    }
}
/// A realistic session: services, the window manager and four user apps.
fn session(n: u64) -> Snapshot {
    let mut s = sample(n * 100, n * 100, n * 37);
    s.managed_pages = 512 * 1024;
    s.allocated_pages = 150_000 + (n * 137 % 30_000);
    s.reads = n * n;
    s.writes = n * 13;
    s.flushes = n;
    let catalog: [(u32, u32, u32); 9] = [
        (1, 1, 0),
        (2, 2, 0),
        (3, 3, 0),
        (4, 4, 0),
        (11, 5, 0),
        (12, 1001, door_abi::platform::mode::DESKTOP as u32),
        (12, 1001, door_abi::platform::mode::TERMINAL as u32),
        (13, 1001, 0),
        (14, 1001, 0),
    ];
    s.count = catalog.len();
    for (i, (manifest, uid, mode)) in catalog.into_iter().enumerate() {
        s.processes[i] = Process {
            pid: 40 + i as u64,
            instance: 9 + i as u64,
            manifest,
            uid,
            mode,
            state: if i == 8 { 2 } else { 3 },
            threads: 1 + (i as u32 % 3),
            samples: n * (i as u64 % 4 + 1),
            private_bytes: (8 + i as u64 * 6) * 1024 * 1024,
        };
    }
    s
}
fn populated() -> Monitor {
    let mut app = Monitor::new();
    for n in 1..=70 {
        let mut s = session(n);
        // Uneven load so the graphs have shape.
        s.busy_samples = (1..=n).map(|k| 10 + (k * 7 % 45)).sum();
        for p in &mut s.processes[..s.count] {
            p.samples = p.samples.min(s.busy_samples / 4);
        }
        app.accept(s);
    }
    app
}
fn fonts(scale: u32) -> FontAtlas {
    FontAtlas::windows_reference_scaled(scale).unwrap()
}
fn render(app: &Monitor, scale: u32, dark: bool, hover: Option<(i32, i32)>) -> Vec<u8> {
    let f = fonts(scale);
    let (w, h) = (app.width, app.height);
    let cache = door_app_ui::ImageCache::default();
    let mut pixels = alloc::vec![0; (w * h * scale * scale * 4) as usize];
    let mut v = View::new_scaled(&mut pixels, w * scale, h * scale, &f, scale).with_images(&cache);
    v.palette.dark = dark;
    v.hover = hover;
    app.draw(&mut v);
    pixels
}
fn save(name: &str, w: u32, h: u32, pixels: &[u8]) {
    let directory = std::path::Path::new("../../target/monitor-review");
    std::fs::create_dir_all(directory).unwrap();
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in pixels.chunks_exact(4) {
        ppm.extend_from_slice(&[p[2], p[1], p[0]]);
    }
    std::fs::write(directory.join(format!("{name}.ppm")), ppm).unwrap();
}
fn center(r: R) -> (i32, i32) {
    (r.x + r.w as i32 / 2, r.y + r.h as i32 / 2)
}
#[test]
fn deltas_use_actual_intervals_and_reject_counter_resets() {
    assert_eq!(
        Monitor::delta(&sample(100, 100, 20), &sample(200, 200, 45)),
        Some((250, 0))
    );
    assert_eq!(
        Monitor::delta(&sample(200, 200, 45), &sample(100, 100, 20)),
        None
    );
    assert_eq!(
        Monitor::delta(&sample(100, 100, 20), &sample(100, 100, 20)),
        None
    );
    let old = sample(1, 0, 0);
    let mut new = sample(2, u64::MAX, u64::MAX);
    new.reads = u64::MAX;
    assert_eq!(Monitor::delta(&old, &new), Some((1000, u64::MAX)));
}
#[test]
fn history_is_time_bounded_and_samples_keep_their_ticks() {
    let mut m = Monitor::new();
    for i in 1..200 {
        m.accept(sample(i * 100, i * 100, i * 10));
    }
    let first = m.history.front().unwrap().ticks;
    let last = m.history.back().unwrap().ticks;
    assert!(last - first <= 6400 && last - first >= 6000);
    assert_eq!(m.history.back().unwrap().cpu, Some(100));
}
#[test]
fn pausing_freezes_and_resuming_starts_a_new_baseline() {
    let mut app = Monitor::new();
    app.accept(sample(100, 100, 20));
    app.accept(sample(200, 200, 60));
    let (previous, current) = (app.previous, app.current);
    app.set_speed(Speed::Paused);
    assert!(!app.sample_due(10_000));
    assert_eq!((app.previous, app.current), (previous, current));
    // Refresh while paused is one shot.
    app.refresh_requested = true;
    assert!(app.sample_due(10_000));
    assert!(!app.sample_due(20_000));
    app.set_speed(Speed::Normal);
    assert!(app.baseline && app.previous.is_none());
    app.accept(sample(1000, 1000, 500));
    assert!(app.history.back().unwrap().cpu.is_none());
    assert!(app.sample_due(0));
    assert!(!app.sample_due(50));
    assert!(app.sample_due(100));
}
#[test]
fn caption_region_is_the_search_box_and_never_the_system_buttons() {
    for width in [240, 360, 490, 640, 900, 1100, 1472, 2560] {
        let mut app = populated();
        app.configure(width, 600);
        let (left, w) = app.caption();
        assert!(w >= 1);
        assert!(left + w <= width.saturating_sub(138));
        if let Some(r) = presentation::search_box(width, 1) {
            assert_eq!((left, w), (r.x as u32, r.w));
            assert_eq!(app.target(r.x + 20, r.y + 16), Some(2));
            assert!(app.text_hit(r.x + 20, r.y + 16));
        }
        // Nothing app-owned under the trusted caption buttons or title text.
        for x in (width as i32 - 138..width as i32).step_by(7) {
            assert_eq!(app.target(x, 20), None);
        }
        assert_eq!(app.target(60, 24), None);
    }
    // The service validates the physical region start (at most 2048 px).
    for scale in 1..=3 {
        for width in [1100, 1920, 2560, 5120] {
            let (left, w) = presentation::caption_region(width, scale);
            assert!(left * scale <= 2048 && w >= 1, "{width} at {scale}x");
        }
    }
}
#[test]
fn navigation_reaches_every_page_at_every_size_and_rail_state() {
    for (w, h) in [(490, 358), (760, 420), (1100, 720), (1472, 952)] {
        for expanded in [None, Some(true), Some(false)] {
            let mut app = populated();
            app.nav_expanded = expanded;
            app.configure(w, h);
            let mut reached = [false; 8];
            for _ in 0..12 {
                for (id, r) in app.hits() {
                    if (10..18).contains(&id) {
                        assert!(r.x + r.w as i32 <= app.rail() as i32);
                        assert_eq!(app.target(center(r).0, center(r).1), Some(id));
                        reached[(id - 10) as usize] = true;
                    }
                }
                app.scroll_at(10, 120, -1);
            }
            assert!(reached.iter().all(|r| *r), "{w}x{h} {expanded:?}");
            // The hamburger toggles labels without changing the page.
            let page = app.page;
            let before = app.rail();
            assert!(app.activate(1));
            assert_ne!(app.rail(), before);
            assert_eq!(app.page, page);
        }
    }
}
#[test]
fn processes_are_grouped_sorted_selectable_and_scroll_with_the_wheel() {
    let mut app = populated();
    app.configure_fonts(1100, 420, &fonts(1));
    let rows = app.rows();
    assert!(matches!(rows[0], Row::Group(Group::Apps, 4)));
    let groups: Vec<_> = rows
        .iter()
        .filter_map(|r| {
            if let Row::Group(g, _) = r {
                Some(*g)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(groups, [Group::Apps, Group::Windows]);
    // Clicking the CPU header sorts largest first, again reverses.
    let cpu = app
        .columns()
        .iter()
        .position(|c| c.key == Key::Cpu)
        .unwrap() as u32;
    assert!(app.activate(50 + cpu));
    assert_eq!(
        app.sorts[0],
        Sort {
            key: Key::Cpu,
            descending: true
        }
    );
    let apps: Vec<u32> = app
        .rows()
        .iter()
        .take_while(|r| !matches!(r, Row::Group(Group::Windows, _)))
        .filter_map(|r| {
            if let Row::Process(p) = r {
                app.cpu(p)
            } else {
                None
            }
        })
        .collect();
    assert!(apps.windows(2).all(|w| w[0] >= w[1]));
    assert!(app.activate(50 + cpu));
    assert!(!app.sorts[0].descending);
    // Row hit test selects exactly the painted process.
    let (id, r) = app.hits().into_iter().find(|(id, _)| *id == 101).unwrap();
    assert_eq!(app.target(center(r).0, center(r).1), Some(id));
    assert!(app.activate(id));
    let Row::Process(p) = app.rows()[1] else {
        panic!()
    };
    assert_eq!(app.selected, Some((p.pid, p.instance)));
    // Wheel over the list scrolls it, bounded by its content.
    let view = app.list_viewport();
    assert!(app.max_scroll() > 0);
    assert!(app.scroll_at(view.x + 50, view.y + 10, -1));
    assert_eq!(app.scroll, 48);
    for _ in 0..50 {
        app.scroll_at(view.x + 50, view.y + 10, -3);
    }
    assert_eq!(app.scroll, app.max_scroll());
    assert!(!app.scroll_at(view.x + 50, view.y + 10, -1));
    while app.scroll_at(view.x + 50, view.y + 10, 2) {}
    assert_eq!(app.scroll, 0);
    // Search filters by name or PID.
    app.search.set("terminal".into(), false);
    assert_eq!(
        app.rows()
            .iter()
            .filter(|r| matches!(r, Row::Process(_)))
            .count(),
        1
    );
    app.search.set("47".into(), false);
    let Row::Process(p) = app.rows()[1] else {
        panic!()
    };
    assert_eq!(p.pid, 47);
}
#[test]
fn disabled_commands_answer_hover_but_do_nothing_and_run_dialog_launches_catalog_apps() {
    let mut app = populated();
    app.configure_fonts(1100, 720, &fonts(1));
    let end = app.hits().into_iter().find(|(id, _)| *id == 21).unwrap().1;
    assert_eq!(app.target(center(end).0, center(end).1), Some(21));
    assert!(!app.activate(21));
    assert!(app.dialog.is_none());
    assert!(app.activate(20));
    assert!(
        app.modal()
            && app.text_hit(
                center(app.dialog_controls()[0]).0,
                center(app.dialog_controls()[0]).1
            )
    );
    for c in "notpad".chars() {
        app.handle_key(((c as u32) << 16) | 30);
    }
    app.submit_run();
    assert!(app.pending_launch.is_none());
    assert!(app.dialog.as_ref().unwrap().error.contains("cannot find"));
    app.dialog
        .as_mut()
        .unwrap()
        .input
        .set("Notepad".into(), false);
    let ok = app.dialog_controls()[1];
    assert_eq!(app.target(center(ok).0, center(ok).1), Some(600));
    assert!(app.activate(600));
    assert_eq!(app.pending_launch, Some(2));
    assert!(app.dialog.as_ref().unwrap().launching);
    assert_eq!(run_target(" TASKMGR "), Some(11));
    assert_eq!(run_target("cmd"), None);
}
#[test]
fn view_menu_changes_speed_and_grouping_and_settings_choose_the_start_page() {
    let mut app = populated();
    app.configure_fonts(1100, 720, &fonts(1));
    let view = app.hits().into_iter().find(|(id, _)| *id == 23).unwrap().1;
    assert!(app.activate(23));
    assert!(app.menu_open());
    // Outside the menu is a dismiss layer; entries answer by index.
    assert_eq!(app.target(400, 600), Some(0));
    let (_, rects) = app.menu_layout().unwrap();
    assert!(rects.iter().all(|r| r.y > view.y));
    let paused = rects[6];
    assert_eq!(app.target(center(paused).0, center(paused).1), Some(506));
    assert!(app.activate(506));
    assert_eq!(app.speed, Speed::Paused);
    assert!(!app.menu_open());
    app.activate(23);
    app.activate(508);
    assert!(!app.group_by_type);
    // Grouping off is one category holding every process (Windows 26H2).
    let rows = app.rows();
    assert_eq!(rows[0], Row::Group(Group::All, 9));
    assert!(rows[1..].iter().all(|r| matches!(r, Row::Process(_))));
    // The optional Isolation column shows only kernel-identity facts.
    assert!(!app.columns().iter().any(|c| c.key == Key::Isolation));
    app.activate(23);
    app.activate(509);
    assert!(app.columns().iter().any(|c| c.key == Key::Isolation));
    let isolation: Vec<&str> = app.processes().iter().map(model::isolation).collect();
    assert_eq!(
        isolation.iter().filter(|s| **s == "System service").count(),
        5
    );
    assert_eq!(
        isolation.iter().filter(|s| **s == "User session").count(),
        4
    );
    app.navigate(SETTINGS);
    let combo = app.settings_controls()[0].1;
    assert_eq!(app.target(center(combo).0, center(combo).1), Some(700));
    app.activate(700);
    let (_, rects) = app.menu_layout().unwrap();
    app.activate(500 + 1);
    assert_eq!(app.default_page, PERFORMANCE);
    assert!(rects.len() == 7);
    let next = app.spawn();
    assert_eq!(next.page, PERFORMANCE);
    assert_eq!(next.speed, Speed::Paused);
}
#[test]
fn every_page_paints_opaque_in_both_themes_and_densities() {
    let mut app = populated();
    app.selected = Some((47, 16));
    for scale in 1..=3 {
        let f = fonts(scale);
        for (w, h) in [(490, 358), (900, 600), (1472, 952)] {
            app.configure_fonts(w, h, &f);
            for page in 0..8 {
                app.page = page;
                for dark in [false, true] {
                    let pixels = render(&app, scale, dark, None);
                    assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
                }
            }
        }
    }
}
/// Review captures for comparison with Windows 11 Task Manager. File names
/// `page-{0..9,20,21}-{light,dark}` are consumed by `xtask reference-apps`.
#[test]
fn review_captures_use_the_actual_painter() {
    let f = fonts(1);
    let mut app = populated();
    app.selected = Some((46, 15));
    app.configure_fonts(1472, 952, &f);
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let mut shots: Vec<(String, Monitor)> = Vec::new();
        for page in 0..8 {
            let mut a = populated();
            a.selected = app.selected;
            a.configure_fonts(1472, 952, &f);
            a.page = page;
            shots.push((format!("page-{page}-{theme}"), a));
        }
        let mut run = populated();
        run.configure_fonts(1472, 952, &f);
        run.open_run_dialog();
        run.dialog
            .as_mut()
            .unwrap()
            .input
            .set("notepad".into(), false);
        shots.push((format!("page-8-{theme}"), run));
        let mut compact = populated();
        compact.configure_fonts(1100, 720, &f);
        compact.nav_expanded = Some(false);
        compact.searching = true;
        compact.search.set("e".into(), false);
        shots.push((format!("page-9-{theme}"), compact));
        for resource in 0..3 {
            let mut perf = populated();
            perf.configure_fonts(1472, 952, &f);
            perf.page = PERFORMANCE;
            perf.resource = resource;
            shots.push((format!("page-{}-{theme}", 20 + resource), perf));
        }
        let mut single = populated();
        single.configure_fonts(1472, 952, &f);
        single.group_by_type = false;
        single.show_isolation = true;
        shots.push((format!("processes-isolation-{theme}"), single));
        let mut menu = populated();
        menu.configure_fonts(1100, 720, &f);
        menu.activate(23);
        shots.push((format!("view-menu-{theme}"), menu));
        for (name, a) in &shots {
            let hover = if name.starts_with("page-0") {
                a.hits()
                    .into_iter()
                    .find(|(id, _)| *id == 21)
                    .map(|(_, r)| center(r))
            } else {
                None
            };
            save(name, a.width, a.height, &render(a, 1, dark, hover));
        }
        let mut perf = populated();
        perf.configure_fonts(1100, 720, &fonts(2));
        perf.page = PERFORMANCE;
        save(
            &format!("performance-2x-{theme}"),
            2200,
            1440,
            &render(&perf, 2, dark, None),
        );
    }
}
/// New windows open at 70% x 78% of the work area (`initial_window`): Task
/// Manager's 1100x720 request becomes 896x524 at 1280x720, 717x562 at
/// 1024x768 and 896x587 at 1280x800. The navigation starts compact there
/// (labels on the hamburger, as Windows does below 1000 px), every page and
/// header command stays inside the window, and the rail reaches every page.
#[test]
fn every_page_lays_out_at_the_default_window_sizes() {
    let f = fonts(1);
    // `initial_window((1280, 720) / (1024, 768) / (1280, 800), (1100, 720))`,
    // asserted by door-graphics-core's placement test.
    for (w, h) in [(896u32, 524u32), (717, 562), (896, 587)] {
        let mut app = populated();
        app.selected = Some((47, 16));
        app.configure_fonts(w, h, &f);
        assert_eq!(app.rail(), 48, "{w}x{h}: compact rail");
        for page in 0..8 {
            app.page = page;
            let pixels = render(&app, 1, false, None);
            assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
            if (w, h) == (896, 524) {
                save(&format!("default-{w}x{h}-page-{page}"), w, h, &pixels);
            }
            for (_, r) in app.hits() {
                assert!(
                    r.x >= 0 && r.x + r.w as i32 <= w as i32,
                    "{w}x{h} page {page}: {r:?}"
                );
            }
        }
    }
}
#[test]
fn every_page_paints_snapped_lines_at_150_percent() {
    use door_workspace::Scale;
    use door_workspace::raster_check::{blurred_lines, crisp_lines};
    let zoom = Scale::from_percent(150).unwrap();
    let atlas = door_workspace::assets::get("fonts/segoe-ui/hinted-150.dfa");
    let f = FontAtlas::from_packed_percent(atlas, 150).unwrap();
    let mut app = populated();
    for page in 0..8 {
        app.page = page;
        for dark in [false, true] {
            let (w, h) = (zoom.len(app.width), zoom.len(app.height));
            let cache = door_app_ui::ImageCache::default();
            let mut pixels = alloc::vec![0; (w * h * 4) as usize];
            let mut v = View::new_zoomed(&mut pixels, w, h, &f, zoom).with_images(&cache);
            assert_eq!((v.width, v.height), (app.width, app.height));
            v.palette.dark = dark;
            app.draw(&mut v);
            let (w, h) = (w as usize, h as usize);
            assert_eq!(blurred_lines(&pixels, w, h), 0, "page {page} dark {dark}");
            assert!(crisp_lines(&pixels, w, h) > 0, "page {page} dark {dark}");
        }
    }
}
/// Windows 11 26H2 Task Manager metrics (docs/design/fidelity-26h2-diff.md
/// T1-T5): 52 px caption with a 260 x 32 search box, hamburger and first
/// destination at a 40 px pitch, the 52..101 page header and its separator,
/// the 51 px two-line column header at 104, 28 px rows and 39 px groups.
#[test]
fn task_manager_metrics_match_the_26h2_reference() {
    assert_eq!(presentation::CAPTION, 52);
    assert_eq!(
        presentation::search_box(1100, 1).map(|r| (r.x, r.y, r.w, r.h)),
        Some((420, 8, 260, 32))
    );
    let mut app = populated();
    app.configure_fonts(1100, 720, &fonts(1));
    let hits = app.hits();
    let at = |id: u32| {
        let r = hits.iter().find(|(i, _)| *i == id).unwrap().1;
        (r.x, r.y, r.w, r.h)
    };
    assert_eq!(at(1), (4, 57, 40, 36));
    assert_eq!(at(10).1, 97);
    assert_eq!(at(11).1 - at(10).1, 40);
    let (_, y, _, h) = at(20);
    assert_eq!((y, h), (60, 32));
    let (_, y, _, h) = at(50);
    assert_eq!((y, h), (104, 51));
    // Group header, then process rows at a 28 px pitch.
    assert!(matches!(app.rows()[0], Row::Group(..)));
    assert_eq!((at(100).1, at(100).3), (155, 39));
    assert_eq!((at(101).1, at(101).3), (194, 28));
    assert_eq!((at(102).1, at(102).3), (222, 28));
    // Revealing the last row scrolls exactly to its bottom.
    let last = app.rows().len() - 1;
    app.reveal_row(last);
    assert_eq!(app.scroll, app.max_scroll());
    for dark in [false, true] {
        let pixels = render(&app, 1, dark, None);
        let px = |x: i32, y: i32| {
            let i = (y as usize * 1100 + x as usize) * 4;
            [pixels[i], pixels[i + 1], pixels[i + 2]]
        };
        let rail = app.rail() as i32;
        // The header separator is one line at y 101 across the layer.
        assert_ne!(px(rail + 200, 101), px(rail + 200, 99), "dark {dark}");
        assert_eq!(px(rail + 200, 99), px(rail + 200, 103), "dark {dark}");
        // Heat cells are accent tints (blue over red), never amber.
        let memory = app.columns().into_iter().find(|c| c.key == Key::Memory);
        let (x, y) = (memory.map_or(0, |c| c.x + 4), at(101).1 + 2);
        let [b, _, r] = px(x, y);
        assert!(b > r, "dark {dark}: heat cell {:?}", px(x, y));
    }
}
