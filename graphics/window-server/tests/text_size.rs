//! Accessibility > Text size (P12-TEXT-SIZE) through the real shell layouts:
//! text-holding rows, bands and buttons grow so every label fits (measured
//! with the text-scaled atlas, ellipsized where Windows trims), icons and
//! chrome keep their size, and the semantic tree and hit testing use the
//! same rectangles the painters draw. 100% stays the identity.
//! A separate test binary: `paint::set_text_percent` is process-wide, so the
//! tests here serialize on `LOCK` and restore 100% when they finish.
use door_abi::clock::WallClock;
use door_accessibility::{Bounds, NodeId};
use door_graphics_core::Rect;
use door_graphics_text::{FontAtlas, TextStyle};
use door_window::menu::{Anchor, Entry, MenuHit};
use door_window::{
    apps::App, clipboard_history as clip, clipboard_view, menus, paint, shell_semantics as sem,
    start, taskbar, tray,
};
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());
/// The applied text size for one test; dropping it restores 100%.
struct TextSize(#[allow(dead_code)] MutexGuard<'static, ()>);
fn text_size(percent: u32) -> TextSize {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    paint::set_text_percent(percent);
    TextSize(guard)
}
impl Drop for TextSize {
    fn drop(&mut self) {
        paint::set_text_percent(100);
    }
}
/// (display %, text %, logical output) cases: maximum text at 100%, and
/// 150% text on a fractional 150% display (1920x1080 physical).
const CASES: [(u32, u32, u32, u32); 2] = [(100, 225, 1920, 1080), (150, 150, 1280, 720)];
fn fonts(display: u32, text: u32) -> FontAtlas {
    FontAtlas::windows_reference_text(display, text).expect("reference atlas")
}
fn width(fonts: &FontAtlas, label: &str, style: TextStyle) -> u32 {
    fonts
        .measure(label, style)
        .expect("measurable")
        .advance_width
}
/// `label` (as drawn) fits `r`: advance width and line box.
fn assert_fits(fonts: &FontAtlas, label: &str, style: TextStyle, r: Rect, what: &str) {
    let w = width(fonts, label, style);
    let h = fonts.line_height(style);
    assert!(w <= r.width(), "{what}: {label:?} is {w} px wide in {r:?}");
    assert!(
        h <= r.height(),
        "{what}: {label:?} line box {h} px in {r:?}"
    );
}
fn inside(inner: Rect, outer: Rect) -> bool {
    inner.x() >= outer.x()
        && inner.y() >= outer.y()
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}
fn bounds(r: Rect) -> Bounds {
    Bounds::from_logical(r.x(), r.y(), r.width() as i32, r.height() as i32)
}
fn center(r: Rect) -> (i32, i32) {
    (r.x() + r.width() as i32 / 2, r.y() + r.height() as i32 / 2)
}

#[test]
fn one_hundred_percent_is_the_identity() {
    let _t = text_size(100);
    assert_eq!(paint::text_grow(32, TextStyle::Regular14), 32);
    assert_eq!(tray::quick_height(), tray::QUICK_HEIGHT);
    assert_eq!(start::header_height(), start::HEADER);
    let mut m = menus::taskbar();
    m.place(
        &fonts(100, 100),
        Anchor::Point(10, 10),
        Rect::new(0, 0, 1920, 1080).unwrap(),
    );
    assert!(m.rows().filter_map(|(_, r)| r).all(|r| r.height() == 32));
    let layout = taskbar::Layout::new(1920, 1080);
    assert_eq!(
        layout.tray(taskbar::TrayTarget::Clock),
        Rect::new(1920 - 112, 1080 - 48 + 4, 104, 40).unwrap()
    );
}

#[test]
fn menu_rows_grow_and_every_label_fits_its_row() {
    for (display, text, w, h) in CASES {
        let _t = text_size(text);
        let fonts = fonts(display, text);
        let area = Rect::new(0, 0, w, h).unwrap();
        for mut m in [
            menus::desktop(false, false),
            menus::desktop(true, false),
            menus::taskbar(),
            menus::jump_list(App::ALL[0], true, 2, true),
        ] {
            let style = if m.style == door_window::menu::Style::Fluent {
                TextStyle::Regular14
            } else {
                TextStyle::Regular12
            };
            let base = if m.style == door_window::menu::Style::Fluent {
                32
            } else {
                26
            };
            m.place(&fonts, Anchor::Point(40, 40), area);
            assert!(inside(m.bounds, area));
            for (i, entry) in m.entries.iter().enumerate() {
                let Entry::Item(item) = entry else { continue };
                let row = m.row(i).unwrap();
                assert_eq!(
                    row.height(),
                    paint::text_grow(base, style),
                    "{text}%: rows grow by the label's line box"
                );
                assert!(row.height() > base);
                let label = m.label_rect(i).unwrap();
                assert!(
                    inside(label, m.bounds)
                        && label.y() == row.y()
                        && label.height() == row.height()
                );
                assert_fits(&fonts, &item.label, style, label, "menu item");
                if let Some(detail) = item.detail {
                    assert!(
                        width(&fonts, &item.label, style)
                            + 16
                            + width(&fonts, detail, TextStyle::Regular12)
                            <= label.width(),
                        "{detail}"
                    );
                }
                // Hit testing uses the painted rows.
                let (x, y) = center(row);
                assert_eq!(m.hit(x, y), MenuHit::Item(0, i));
            }
        }
    }
}

#[test]
fn start_tiles_rows_and_semantics_follow_the_grown_layout() {
    for (display, text, w, h) in CASES {
        let _t = text_size(text);
        let fonts = fonts(display, text);
        let layout = taskbar::Layout::new(w, h);
        let mut s = start::Start::default();
        s.open();
        s.recent = vec![
            start::FileResult {
                folder: false,
                name: "Quarterly report.docx".into(),
                path: r"C:\Users\1001\Documents".into(),
            },
            start::FileResult {
                folder: true,
                name: "Projects".into(),
                path: r"C:\Users\1001".into(),
            },
        ];
        for view in [start::View::Category, start::View::List, start::View::Grid] {
            s.view = view;
            let g = s.geometry(layout);
            assert!(g.search_box().height() >= fonts.line_height(TextStyle::Regular14));
            assert_fits(
                &fonts,
                "Pinned",
                TextStyle::Semibold14,
                g.pinned_header(0),
                "section header",
            );
            for (i, app) in s.pins.iter().enumerate() {
                let Some(r) = s.rect(&g, &fonts, start::Target::Pinned(i)) else {
                    continue;
                };
                assert_eq!(
                    r.height(),
                    paint::text_grow(80, TextStyle::Regular12),
                    "tiles grow by their label"
                );
                let band = start::tile_label(r);
                assert!(inside(band, r));
                let label =
                    paint::ellipsize(&fonts, app.label(), TextStyle::Regular12, band.width() - 8);
                assert_fits(
                    &fonts,
                    &label,
                    TextStyle::Regular12,
                    band,
                    "Start tile label",
                );
            }
            for i in 0..g.recent_count() {
                let r = g.recent_item(i, 0).unwrap();
                assert!(
                    r.height()
                        >= 7 + fonts.line_height(TextStyle::Regular14)
                            + fonts.line_height(TextStyle::Regular12)
                );
            }
            if view == start::View::List {
                for (row, r) in g.list(0) {
                    if let start::ListRow::App(app) = row {
                        assert_fits(
                            &fonts,
                            app.label(),
                            TextStyle::Regular14,
                            Rect::new(r.x() + 48, r.y(), r.width() - 56, r.height()).unwrap(),
                            "All list row",
                        );
                    }
                }
            }
            let name_w = width(&fonts, start::USER_NAME, TextStyle::Regular14);
            let user = g.user_button(name_w);
            assert!(
                inside(user, g.footer())
                    && user.height() >= fonts.line_height(TextStyle::Regular14)
            );
            assert!(inside(g.power_button(), g.footer()));
            // Semantics and hit testing use the same rectangles.
            let tree = sem::start(1, &s, layout, &fonts);
            tree.validate().unwrap();
            let body = g.body();
            for target in s.targets(&g) {
                let Some(r) = s.rect(&g, &fonts, target) else {
                    continue;
                };
                let node = tree.get(NodeId(sem::start_node(target).unwrap())).unwrap();
                assert_eq!(node.bounds, bounds(r), "{target:?}");
                let (x, y) = center(r);
                let in_body = !matches!(
                    target,
                    start::Target::Search | start::Target::User | start::Target::Power
                );
                if !in_body || body.contains(x, y) {
                    assert_eq!(s.hit(&g, &fonts, x, y), Some(target), "{text}% {view:?}");
                }
            }
        }
        // Search results: grown rows hold their text.
        s.type_text("notes");
        let g = s.geometry(layout);
        // Seven 26H2 pills: trailing ones that do not fit the grown row are
        // left out (not drawn, hit or announced); All..Documents always fit.
        for (i, f) in door_window::search::Filter::ALL.into_iter().enumerate() {
            let chip = g.chip(&fonts, f);
            assert_fits(&fonts, f.label(), TextStyle::Regular14, chip, "search chip");
            let shown = s.rect(&g, &fonts, start::Target::Chip(f)).is_some();
            assert_eq!(shown, g.chip_fits(&fonts, f));
            assert!(i > 2 || shown, "{text}%: {f:?} hidden");
            if !shown {
                continue;
            }
            assert!(
                chip.right() <= g.panel.right() - 16,
                "{text}%: chip {f:?} {chip:?} leaves {:?}",
                g.panel
            );
        }
        for (row, r) in s.result_layout(&g) {
            if let start::Row::Result(i) = row {
                assert!(
                    r.height() >= fonts.line_height(TextStyle::Regular14) + 8,
                    "result {i}: {r:?}"
                );
                let node = sem::start(2, &s, layout, &fonts)
                    .get(NodeId(sem::start_node(start::Target::Result(i)).unwrap()))
                    .map(|n| n.bounds);
                assert_eq!(node, Some(bounds(r)));
            }
        }
    }
}

#[test]
fn flyout_rows_fit_their_text_and_semantics_match() {
    for (display, text, w, h) in CASES {
        let _t = text_size(text);
        let fonts = fonts(display, text);
        let layout = taskbar::Layout::new(w, h);
        let quick = tray::quick_rect(layout);
        assert_eq!(
            quick.height(),
            tray::quick_height().min(layout.work_area().height() - 24)
        );
        for (i, (_, label, _)) in tray::TILES.iter().enumerate() {
            let band = tray::tile_label(layout, i);
            assert!(band.y() >= tray::tile(layout, i).bottom() && inside(band, quick));
            if i < 3 {
                assert!(
                    band.bottom() <= tray::tile(layout, i + 3).y(),
                    "caption clear of the next tile row"
                );
            }
            let shown = paint::ellipsize(&fonts, label, TextStyle::Regular12, band.width());
            assert_fits(
                &fonts,
                &shown,
                TextStyle::Regular12,
                band,
                "quick settings tile",
            );
        }
        for row in 0..2 {
            let caption = tray::slider_caption(layout, row);
            assert!(
                caption.y() >= tray::slider_row(layout, row).y() + 12 && inside(caption, quick)
            );
            assert!(caption.height() >= fonts.line_height(TextStyle::Regular12));
            assert!(caption.bottom() <= tray::settings_button(layout).y() - 8);
        }
        let state = tray::State {
            open: Some(tray::Flyout::Devices),
            ..Default::default()
        };
        let tree = sem::flyout(1, &state, layout, None).unwrap();
        for i in 0..tray::TILES.len() {
            assert_eq!(
                tree.get(NodeId(100 + i as u64)).unwrap().bounds,
                bounds(tray::tile(layout, i))
            );
        }
        for row in 0..2 {
            assert_eq!(
                tree.get(NodeId(150 + row as u64)).unwrap().bounds,
                bounds(tray::slider_row(layout, row))
            );
        }
        // Calendar and notification center bands.
        let (header, month, prev, next) = tray::calendar_controls(layout);
        assert!(header.height() >= fonts.line_height(TextStyle::Semibold14) + 8);
        assert!(
            month.height() >= fonts.line_height(TextStyle::Semibold14)
                && inside(prev, month)
                && inside(next, month)
        );
        if let Some(n) = tray::notifications_rect(layout) {
            let (heading, line) = tray::notifications_rows(n);
            assert_fits(
                &fonts,
                "Notifications",
                TextStyle::Semibold14,
                heading,
                "notification heading",
            );
            assert_fits(
                &fonts,
                "No new notifications",
                TextStyle::Regular14,
                line,
                "notification line",
            );
            assert!(inside(line, n));
        }
        // Input switcher rows hold the language over the keyboard name.
        let row = tray::input_row(layout, 2, 0);
        let (abbr, language, keyboard) = tray::input_row_text(&fonts, row, "ENG");
        assert!(width(&fonts, "ENG", TextStyle::Semibold14) <= abbr.width());
        assert_fits(
            &fonts,
            "English (United States)",
            TextStyle::Regular14,
            language,
            "switcher language",
        );
        assert_fits(
            &fonts,
            "US keyboard",
            TextStyle::Regular12,
            keyboard,
            "switcher keyboard",
        );
        assert!(keyboard.y() >= language.bottom() - 4 && keyboard.bottom() <= row.bottom());
    }
}

#[test]
fn taskbar_keeps_its_height_and_widens_the_clock() {
    let _t = text_size(225);
    let fonts = fonts(100, 225);
    let layout = taskbar::Layout::new(1920, 1080);
    let bar = layout.bar();
    assert_eq!(bar.height(), 48, "the taskbar itself does not grow");
    let clock = layout.tray(taskbar::TrayTarget::Clock);
    let time = WallClock {
        year: 2026,
        month: 12,
        day: 28,
        hour: 22,
        minute: 58,
        second: 0,
    };
    let (time_text, date_text) = taskbar::clock_text(time, 0);
    for line in [&time_text, &date_text] {
        assert!(
            width(&fonts, line, TextStyle::Regular12) <= clock.width() - 32,
            "{line}"
        );
    }
    let quick = layout.tray(taskbar::TrayTarget::QuickSettings);
    let language = layout.tray(taskbar::TrayTarget::Language);
    assert!(language.right() <= quick.x() && quick.right() <= clock.x() && inside(clock, bar));
    assert_eq!(
        (quick.width(), quick.height()),
        (60, 40),
        "glyph targets keep their size"
    );
}

#[test]
fn candidate_window_and_clipboard_history_grow_with_text() {
    use door_service_protocol::window::Rect as WRect;
    use door_window::text_input::candidate::{CandidateWindow, Role};
    for text in [150, 225] {
        let _t = text_size(text);
        let fonts = fonts(100, text);
        let c = CandidateWindow {
            serial: 1,
            items: vec!["你好".into(), "拟好".into()],
            selected: 0,
            page: 0,
            pages: 2,
            vertical: false,
            anchor: WRect {
                x: 300,
                y: 100,
                width: 2,
                height: 30,
            },
            scale: 100,
        };
        let measure = |s: &str, px: u32| s.chars().count() as u32 * px;
        let l = c.layout(
            WRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            &measure,
        );
        let cand = door_graphics_text::text_len(16, text) as i32;
        assert_eq!(l.frame.height, paint::text_grow(36, TextStyle::Regular16));
        for r in l.runs.iter().filter(|r| r.role == Role::Candidate) {
            assert!(r.top >= l.frame.y && r.top + cand <= l.frame.y + l.frame.height as i32);
        }
        let v = CandidateWindow {
            vertical: true,
            ..c.clone()
        };
        let l = v.layout(
            WRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            &measure,
        );
        assert!(l.cells.iter().all(|cell| cell.height as i32 >= cand));
        // Clipboard history: cards, menu rows, the off state and its button.
        let g = clip::Geometry::at(Rect::new(100, 100, clip::WIDTH, clip::HEIGHT).unwrap());
        let card = g.card(0, 0);
        assert_eq!(card.height(), clip::card_height());
        assert!(
            12 + 2 * clip::card_line() <= card.height() + 12
                && clip::card_line() >= fonts.line_height(TextStyle::Regular14)
        );
        assert!(width(&fonts, clip::CLEAR_ALL, TextStyle::Regular14) <= g.clear.width());
        assert!(g.clear.height() >= fonts.line_height(TextStyle::Regular14));
        let row = g.menu_row(0, 0, 1);
        assert_fits(
            &fonts,
            "Delete",
            TextStyle::Regular14,
            Rect::new(row.x() + 40, row.y(), row.width() - 48, row.height()).unwrap(),
            "clipboard menu",
        );
        for body in [clip::OFF_BODY, clip::EMPTY_BODY] {
            let lines = clipboard_view::wrap_lines(&fonts, body, clip::WIDTH - 64);
            assert!(
                lines.len() as u32 <= clip::body_lines(),
                "{text}%: {lines:?}"
            );
        }
        let (title, body_top) = clip::state_title(g.panel, 140);
        assert!(title.height() >= fonts.line_height(TextStyle::Semibold14));
        let turn_on = g.turn_on();
        assert!(turn_on.y() >= body_top + (clip::body_lines() * clip::body_pitch()) as i32);
        assert!(
            inside(turn_on, g.panel)
                && width(&fonts, clip::TURN_ON, TextStyle::Regular14) <= turn_on.width()
        );
    }
}
