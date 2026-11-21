//! Windows 11 26H2 taskbar Search: the coloured search glyph (ring sweep
//! stops, handle, frosted disc, contrast fallback, press feedback) at 100 %
//! and 150 %, and the dedicated Search panel (layout at 100/125/150 %,
//! semantics, Start hand-over and dismissal) through the shell painters.
use door_graphics_core::{Canvas, Rect, Scale};
use door_graphics_text::FontAtlas;
use door_service_protocol::desktop::{ContrastTheme, Controls};
use door_window::{
    paint::{self, Palette},
    search::Filter,
    search_glyph::{self as glyph, HANDLE, MID},
    shell_semantics as sem,
    start::{self, Activation, FileResult, Mode, Start, Target},
    taskbar,
};

const TASKBAR: [u8; 3] = [0x2E, 0x20, 0x13];

/// A `percent` canvas filled with the owner's dark taskbar colour and the
/// glyph drawn at logical (10, 10) in a 24 DIP box.
fn render(percent: u32, palette: &Palette, permille: u32) -> (Vec<u8>, usize) {
    let w = 64usize;
    let mut bytes = vec![0u8; w * w * 4];
    for p in bytes.chunks_exact_mut(4) {
        p.copy_from_slice(&[TASKBAR[2], TASKBAR[1], TASKBAR[0], 255]);
    }
    let mut c = Canvas::new(&mut bytes, w as u32, w as u32, w * 4).unwrap();
    c.set_scale(Scale::from_percent(percent).unwrap()).unwrap();
    glyph::draw(&mut c, 10, 10, 24, palette, permille);
    (bytes, w)
}
/// Physical pixel holding design point (`dx`, `dy`) (DIP x256 in the 24 DIP
/// box at logical (10, 10)), as sRGB.
fn sample(bytes: &[u8], w: usize, percent: u32, dx: i64, dy: i64) -> [u8; 3] {
    let p = i64::from(percent);
    let x = ((10 * 256 + dx) * p / 100 / 256) as usize;
    let y = ((10 * 256 + dy) * p / 100 / 256) as usize;
    let at = (y * w + x) * 4;
    [bytes[at + 2], bytes[at + 1], bytes[at]]
}
fn near(a: [u8; 3], b: [u8; 3], tolerance: i32) -> bool {
    (0..3).all(|i| (i32::from(a[i]) - i32::from(b[i])).abs() <= tolerance)
}

#[test]
fn glyph_ring_stops_handle_and_disc_match_the_26h2_capture() {
    let palette = Palette::themed(true);
    const C: i64 = 2688; // ring centre, 10.5 DIP
    for percent in [100, 150] {
        let (bytes, w) = render(percent, &palette, 1000);
        let at = |dx: i64, dy: i64| sample(&bytes, w, percent, dx, dy);
        // Clock positions on the ring's centre line.
        let ring = |degrees: f64| {
            let a = degrees.to_radians();
            at(
                C + (MID as f64 * a.sin()) as i64,
                C - (MID as f64 * a.cos()) as i64,
            )
        };
        let cases: [(f64, [u8; 3], i32); 6] = [
            (0.0, [0x2E, 0xAB, 0xA7], 12),   // cyan near 12 o'clock
            (60.0, [0x5E, 0xE1, 0x78], 14),  // green at 2 o'clock
            (90.0, [0x3E, 0xBE, 0x97], 12),  // teal down the right
            (180.0, [0x00, 0x78, 0xD3], 3),  // solid blue at the bottom
            (270.0, [0x00, 0x78, 0xD3], 3),  // ... and the left
            (320.0, [0x0B, 0x84, 0xC8], 10), // shifting towards cyan
        ];
        for (degrees, expected, tolerance) in cases {
            let got = ring(degrees);
            assert!(
                near(got, expected, tolerance),
                "{percent}% ring at {degrees}°: {got:02X?} vs {expected:02X?}"
            );
        }
        // Handle: solid #0077D4 on its centre line near the corner.
        let handle = at(20 * 256, 20 * 256);
        assert!(near(handle, HANDLE, 3), "{percent}% handle {handle:02X?}");
        // Frosted disc: white ~16 % over #2E2013 reads about #504840.
        let disc = at(C, C);
        assert!(
            near(disc, [0x50, 0x46, 0x3B], 6),
            "{percent}% disc {disc:02X?}"
        );
        // Outside the glyph the taskbar is untouched.
        assert_eq!(at(23 * 256, 256), TASKBAR, "{percent}% outside");
        // Antialiased edge: some pixel between the taskbar and the ring blue.
        let edge = (0..w * w).any(|i| {
            let p = [bytes[i * 4 + 2], bytes[i * 4 + 1], bytes[i * 4]];
            p[2] > TASKBAR[2] + 20 && p[2] < 0xC0 && p[0] < 0x30
        });
        assert!(edge, "{percent}%: no antialiased ring edge");
    }
}

#[test]
fn glyph_is_monochrome_in_contrast_themes_and_shrinks_while_pressed() {
    for theme in ContrastTheme::ALL {
        let palette = Palette::contrast_theme(theme);
        let text = [palette.text[2], palette.text[1], palette.text[0]];
        for percent in [100, 150] {
            let (bytes, w) = render(percent, &palette, 1000);
            let mut inked = 0;
            for i in 0..w * w {
                let p = [bytes[i * 4 + 2], bytes[i * 4 + 1], bytes[i * 4]];
                if p == TASKBAR {
                    continue;
                }
                inked += 1;
                // Every inked pixel lies between the background and the
                // system text colour (no sweep colours).
                for ch in 0..3 {
                    let (lo, hi) = (TASKBAR[ch].min(text[ch]), TASKBAR[ch].max(text[ch]));
                    assert!(
                        (lo.saturating_sub(1)..=hi.saturating_add(1)).contains(&p[ch]),
                        "{theme:?} {percent}%: {p:02X?} outside {TASKBAR:02X?}..{text:02X?}"
                    );
                }
            }
            assert!(inked > 20, "{theme:?} {percent}%: glyph missing");
        }
    }
    // Press feedback: the inked box shrinks about its centre.
    let palette = Palette::themed(true);
    let extent = |permille: u32| {
        let (bytes, w) = render(150, &palette, permille);
        let xs: Vec<usize> = (0..w * w)
            .filter(|i| [bytes[i * 4 + 2], bytes[i * 4 + 1], bytes[i * 4]] != TASKBAR)
            .map(|i| i % w)
            .collect();
        (*xs.iter().min().unwrap(), *xs.iter().max().unwrap())
    };
    let (rest, pressed) = (extent(1000), extent(glyph::PRESSED_PERMILLE));
    assert!(
        pressed.0 > rest.0 && pressed.1 < rest.1,
        "{rest:?} vs {pressed:?}"
    );
}

#[test]
fn taskbar_search_button_and_box_draw_the_coloured_glyph() {
    let blue = [0x00, 0x78, 0xD3];
    for (percent, icon_only) in [(100, true), (150, true), (150, false)] {
        let scale = Scale::from_percent(percent).unwrap();
        let (pw, ph) = (1920u32, 1080u32);
        let (lw, lh) = (scale.logical_len(pw), scale.logical_len(ph));
        let controls = Controls::DEFAULT.with(Controls::SEARCH_ICON_ONLY, icon_only);
        let layout = taskbar::Layout::with_controls(lw, lh, controls);
        let fonts = FontAtlas::windows_reference_text(percent, 100).unwrap();
        let mut bytes = vec![255u8; (pw * ph * 4) as usize];
        let mut c = Canvas::new(&mut bytes, pw, ph, pw as usize * 4).unwrap();
        c.set_scale(scale).unwrap();
        taskbar::draw(
            &mut c,
            &fonts,
            layout,
            &taskbar::View {
                buttons: &[],
                pointer: (-1, -1),
                pressed: None,
                press: None,
                hover: [(None, 0); 2],
                start_open: false,
                search_open: false,
                task_view_open: false,
                quick_open: false,
                calendar_open: false,
                hidden_icons: None,
                time: None,
                region: 0,
                overrides: 0,
                network: taskbar::Net::Connected,
                vpn: false,
                audio: None,
                dark: true,
                material: None,
            },
        );
        let slot = paint::physical(layout.slot(taskbar::Slot::Search, 0), scale);
        let blues = (slot.y()..slot.bottom())
            .flat_map(|y| (slot.x()..slot.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let at = (y as usize * pw as usize + x as usize) * 4;
                near([bytes[at + 2], bytes[at + 1], bytes[at]], blue, 4)
            })
            .count();
        assert!(
            blues > 8,
            "{percent}% icon_only={icon_only}: {blues} blue px"
        );
    }
}

fn with_recent(s: &mut Start, n: usize) {
    s.recent = (0..n)
        .map(|i| FileResult {
            folder: i % 3 == 0,
            name: format!("Recent {i}.txt"),
            path: format!(r"C:\Users\1001\Documents\Recent {i}.txt"),
        })
        .collect();
}

#[test]
fn search_panel_layout_at_100_125_and_150_percent() {
    // The owner's outputs: 1920x1080 at 100 %, 1920x1080 at 125 %, and the
    // 2560x1600 laptop at 150 %.
    for (percent, pw, ph) in [(100, 1920, 1080), (125, 1920, 1080), (150, 2560, 1600)] {
        let scale = Scale::from_percent(percent).unwrap();
        let (lw, lh) = (scale.logical_len(pw), scale.logical_len(ph));
        let layout = taskbar::Layout::new(lw, lh);
        let fonts = FontAtlas::windows_reference_text(percent, 100).unwrap();
        let mut s = Start::default();
        s.open_mode(Mode::Search);
        with_recent(&mut s, 5);
        let g = s.geometry(layout);
        let start_g = start::Geometry::new(layout);
        let area = layout.work_area();
        // Same anchor as Start: centred above the taskbar, 12 px margins.
        assert!(g.search, "{percent}%");
        assert_eq!(
            (g.panel.x(), g.panel.width()),
            (start_g.panel.x(), start_g.panel.width())
        );
        assert_eq!(g.panel.bottom(), area.bottom() - 12, "{percent}%");
        let max = (area.height() as i32 - 24).clamp(200, 866);
        assert_eq!(
            g.panel.height() as i32,
            start::SEARCH_HEIGHT.min(max),
            "{percent}%"
        );
        // No account/Power footer: the body runs to the panel bottom.
        assert_eq!(g.body().bottom(), g.panel.bottom());
        let targets = s.targets(&g);
        assert!(!targets.contains(&Target::Power) && !targets.contains(&Target::User));
        // 26H2: no filter pills before typing.
        assert!(!targets.iter().any(|t| matches!(t, Target::Chip(_))));
        // Recent (left); Quick searches over Top apps (right), in the body.
        let body = g.body();
        let recent = g.home_recent_header();
        let quick = g.quick_header();
        let card = g.quick_card();
        let top = g.top_apps_header();
        assert!(recent.right() <= quick.x() && recent.y() == quick.y());
        assert!(card.y() > quick.y() && top.y() > card.bottom());
        assert_eq!((card.x(), top.x()), (quick.x(), quick.x()));
        for (i, (_, page)) in start::QUICK_SEARCHES.iter().enumerate() {
            let r = s.rect(&g, &fonts, Target::QuickSearch(i)).unwrap();
            assert!(
                card.contains(r.x(), r.y()) && r.right() <= card.right(),
                "{percent}% {r:?}"
            );
            assert!(targets.contains(&Target::QuickSearch(i)));
            assert_eq!(
                s.hit(&g, &fonts, r.x() + 4, r.y() + r.height() as i32 / 2),
                Some(Target::QuickSearch(i))
            );
            assert_eq!(
                s.activate(Target::QuickSearch(i)),
                Activation::Settings(*page)
            );
        }
        assert_eq!(s.home_recent_count(&g), 5);
        for i in 0..5 {
            let r = s.rect(&g, &fonts, Target::Recent(i)).unwrap();
            assert!(r.x() == recent.x() && r.bottom() <= body.bottom(), "{r:?}");
            assert_eq!(
                s.hit(&g, &fonts, r.x() + 4, r.y() + 4),
                Some(Target::Recent(i))
            );
            assert_eq!(s.activate(Target::Recent(i)), Activation::OpenRecent(i));
        }
        for i in 0..start::TOP_APPS {
            let r = s.rect(&g, &fonts, Target::TopApp(i)).unwrap();
            assert!(
                r.x() >= top.x() && r.right() <= top.right(),
                "{percent}% {r:?}"
            );
            assert!(r.bottom() <= body.bottom());
            assert_eq!(s.activate(Target::TopApp(i)), Activation::Launch(s.pins[i]));
        }
        // Painted at the scale: the opaque search pill, a Top apps icon,
        // and no Power glyph where Start's footer would be.
        let mut bytes = vec![0u8; (pw * ph * 4) as usize];
        for p in bytes.chunks_exact_mut(4) {
            p.copy_from_slice(&[40, 40, 40, 255]);
        }
        let mut c = Canvas::new(&mut bytes, pw, ph, pw as usize * 4).unwrap();
        c.set_scale(scale).unwrap();
        start::draw(
            &mut c,
            &fonts,
            &s,
            layout,
            &start::Frame {
                pointer: (-1, -1),
                dark: true,
                slide: 0,
                transparency: false,
                taskbar_pins: &[],
            },
        );
        let px = |x: i32, y: i32| {
            let at = (scale.px(y) as usize * pw as usize + scale.px(x) as usize) * 4;
            [bytes[at + 2], bytes[at + 1], bytes[at]]
        };
        let palette = Palette::themed(true);
        let field = [palette.field[2], palette.field[1], palette.field[0]];
        let pill = g.search_box();
        assert_eq!(
            px(pill.right() - 24, pill.y() + 16),
            field,
            "{percent}% pill"
        );
        let tile = s.rect(&g, &fonts, Target::TopApp(0)).unwrap();
        let surface = [palette.surface[2], palette.surface[1], palette.surface[0]];
        assert_ne!(
            px(tile.x() + 44, tile.y() + 24),
            surface,
            "{percent}% top app icon"
        );
        let power = start_g.power_button();
        let centre = (power.x() + power.width() as i32 / 2, g.panel.bottom() - 32);
        assert_eq!(px(centre.0, centre.1), surface, "{percent}% footer area");
        // Typing shows the pills in Windows order, 8 px apart.
        s.type_text("notes");
        let g = s.geometry(layout);
        let targets = s.targets(&g);
        let mut previous: Option<Rect> = None;
        for f in Filter::ALL {
            assert!(g.chip_fits(&fonts, f), "{percent}% {f:?}");
            let r = g.chip(&fonts, f);
            if let Some(p) = previous {
                assert_eq!(r.x(), p.right() + 8);
            }
            previous = Some(r);
            assert!(targets.contains(&Target::Chip(f)));
        }
    }
}

#[test]
fn search_panel_semantics_mirror_start_without_the_footer() {
    let layout = taskbar::Layout::new(1920, 1080);
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let mut s = Start::default();
    s.open_mode(Mode::Search);
    with_recent(&mut s, 2);
    let t = sem::start(1, &s, layout, &fonts);
    t.validate().unwrap();
    let o = door_accessibility::outline(&t);
    assert!(o.starts_with("window \"Search\""), "{o}");
    // Its search box is a control of its own (announced after Start).
    let id = sem::start_node::SEARCH_PANEL;
    assert!(t.get(door_accessibility::NodeId(id)).is_some());
    assert_eq!(
        sem::start_target(&s, &s.geometry(layout), id),
        Some(Target::Search)
    );
    assert_eq!(sem::panel_node(&s, Target::Search), Some(id));
    assert!(o.contains("edit \"Search box\""), "{o}");
    // The home: no pills, the Quick searches card of Settings links.
    assert!(!o.contains("tab \"Search filters\""), "{o}");
    assert!(o.contains("list \"Quick searches\""), "{o}");
    assert!(o.contains("button \"Display settings\""), "{o}");
    assert!(o.contains("list \"Recent\""), "{o}");
    assert!(o.contains("list item \"Recent 1.txt\""), "{o}");
    assert!(o.contains("list \"Top apps\""), "{o}");
    // Tab walks Recent, Quick searches, Top apps.
    {
        let g = s.geometry(layout);
        let mut probe = Start::default();
        probe.open_mode(Mode::Search);
        with_recent(&mut probe, 2);
        let mut seen = vec![];
        for _ in 0..12 {
            probe.navigate(&g, &fonts, 4);
            seen.push(probe.focus.unwrap());
        }
        assert!(!seen.iter().any(|t| matches!(t, Target::Chip(_))));
        assert!(seen.contains(&Target::Recent(1)) && seen.contains(&Target::QuickSearch(0)));
    }
    // Typing: the filter pills.
    s.type_text("notes");
    let t = sem::start(1, &s, layout, &fonts);
    t.validate().unwrap();
    let o = door_accessibility::outline(&t);
    assert!(o.contains("tab \"Search filters\""), "{o}");
    for label in ["All", "Apps", "Documents", "Settings", "Folders"] {
        assert!(o.contains(&format!("tab item \"{label}\"")), "{label}: {o}");
    }
    assert!(
        o.lines()
            .any(|l| l.contains("tab item \"Web\"") && l.contains("DISABLED")),
        "{o}"
    );
    assert!(
        o.lines()
            .any(|l| l.contains("tab item \"Photos\"") && l.contains("DISABLED")),
        "{o}"
    );
    assert!(
        !o.contains("\"Power\"") && !o.contains(start::USER_NAME),
        "{o}"
    );
    // The taskbar announces Search as an expandable button, expanded now.
    let buttons = taskbar::buttons(&[], &[]);
    let tray = door_window::tray::State::default();
    let bar = sem::taskbar(1, layout, &buttons, &tray, Some(Mode::Search));
    let o = door_accessibility::outline(&bar);
    assert!(
        o.lines()
            .any(|l| l.contains("button \"Search\"") && l.contains("EXPANDED")),
        "{o}"
    );
    // With a query, Tab walks the pills (skipping disabled Web/Photos).
    let g = s.geometry(layout);
    let mut seen = vec![];
    for _ in 0..12 {
        s.navigate(&g, &fonts, 4);
        seen.push(s.focus.unwrap());
    }
    assert!(seen.contains(&Target::Chip(Filter::All)));
    assert!(!seen.contains(&Target::Chip(Filter::Web)));
    assert!(!seen.contains(&Target::Chip(Filter::Photos)));
}

#[test]
fn search_panel_dismisses_and_start_hands_over_when_typing() {
    let layout = taskbar::Layout::new(1920, 1080);
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    // The dedicated panel: Esc closes it, even with a query.
    let mut s = Start::default();
    s.open_mode(Mode::Search);
    assert!(s.escape());
    s.open_mode(Mode::Search);
    s.type_text("notes");
    assert!(s.search_panel() && s.searching());
    assert!(s.escape(), "Esc dismisses the Search panel");
    // Clicking outside: no target (the shell closes the panel), inside on
    // nothing: the panel itself.
    let g = s.geometry(layout);
    assert_eq!(s.hit(&g, &fonts, g.panel.x() - 5, g.panel.y() + 40), None);
    assert_eq!(
        s.hit(&g, &fonts, g.panel.x() + 4, g.panel.bottom() - 4),
        Some(Target::Panel)
    );
    // Typing in Start switches to the same Search experience; Esc (or
    // emptying the query) returns to Start, a second Esc closes.
    let mut s = Start::default();
    s.open();
    assert_eq!(s.mode, Mode::Start);
    s.type_text("set");
    assert!(s.search_panel() && s.from_start);
    let g = s.geometry(layout);
    assert!(g.search, "no footer while searching");
    assert!(!s.targets(&g).contains(&Target::Power));
    assert!(!s.escape());
    assert_eq!(s.mode, Mode::Start);
    assert!(s.escape());
    s.open();
    s.type_text("x");
    assert!(s.backspace());
    assert_eq!(s.mode, Mode::Start);
}
