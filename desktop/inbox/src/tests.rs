extern crate std;
use crate::calculator::{Calculator, DOCK, PANE, TITLE};
use door_app_ui::{ImageCache, ui};
use door_graphics_text::FontAtlas;

fn paint(c: &Calculator, dark: bool, hover: Option<(i32, i32)>) -> alloc::vec::Vec<u8> {
    let fonts = FontAtlas::windows_reference_scaled(1)
        .unwrap()
        .with_unicode(
            door_workspace::assets::text_system().unwrap(),
            door_graphics_text::Family::Ui,
        );
    let (w, h) = (c.width, c.height);
    let mut px = alloc::vec![0; (w * h * 4) as usize];
    let images = ImageCache::default();
    let mut v = ui::View::new_scaled(&mut px, w, h, &fonts, 1).with_images(&images);
    v.palette.dark = dark;
    v.hover = hover;
    c.paint(&mut v);
    px
}

#[test]
fn the_keypad_fills_the_window_below_the_display_without_overlap() {
    for (w, h) in [(322, 500), (500, 700), (1200, 940)] {
        let mut c = Calculator::new();
        door_app_ui::Application::configure(&mut c, w, h);
        let column = if w >= DOCK { w - PANE } else { w };
        let keys: alloc::vec::Vec<_> = (0..24).map(|i| c.key_rect(i)).collect();
        for (i, a) in keys.iter().enumerate() {
            assert!(
                a.x >= 4 && a.x + a.w as i32 <= column as i32 - 4,
                "{w}x{h} key {i}"
            );
            assert!(
                a.y > TITLE && a.y + a.h as i32 <= h as i32 - 4,
                "{w}x{h} key {i}"
            );
            for b in &keys[i + 1..] {
                let apart = a.x + a.w as i32 <= b.x
                    || b.x + b.w as i32 <= a.x
                    || a.y + a.h as i32 <= b.y
                    || b.y + b.h as i32 <= a.y;
                assert!(apart, "{w}x{h}: keys overlap");
            }
        }
        // Keys are 2 px apart.
        assert_eq!(keys[1].x - (keys[0].x + keys[0].w as i32), 2);
        // The memory row sits above the keypad.
        assert!(c.memory_rect(0).y + c.memory_rect(0).h as i32 <= keys[0].y);
        assert_eq!(c.docked(), w >= DOCK);
    }
}

#[test]
fn clicking_keys_runs_the_calculation_and_history_docks_when_wide() {
    let mut c = Calculator::new();
    door_app_ui::Application::configure(&mut c, 322, 500);
    let press = |c: &mut Calculator, label: &str| {
        let i = crate::calculator::key_labels(c)
            .iter()
            .position(|(n, _, _)| n == label)
            .unwrap();
        let r = c.key_rect(i);
        let t = door_app_ui::Application::target(c, r.x + 4, r.y + 4).unwrap();

        assert!(c.activate(t));
    };
    for k in ["7", "Plus", "8", "Equals"] {
        press(&mut c, k);
    }
    assert_eq!(c.engine.display(), "15");
    assert_eq!(c.engine.history.len(), 1);
    door_app_ui::Application::configure(&mut c, 1200, 800);
    assert!(c.docked());
}

/// Review renders (`DUMP_DIR=<dir> cargo test -p door-inbox render_calculator
/// -- --ignored`): BMPs of the default and wide windows in both themes.
#[test]
#[ignore]
fn render_calculator() {
    fn bmp(path: &str, w: u32, h: u32, bgra: &[u8]) {
        let mut out = std::vec::Vec::new();
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(54 + w * h * 4).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(-(h as i32)).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        out.extend_from_slice(bgra);
        std::fs::write(path, out).unwrap();
    }
    let dir = std::env::var("DUMP_DIR").unwrap();
    let mut c = Calculator::new();
    door_app_ui::Application::configure(&mut c, 322, 500);
    for k in [
        crate::calculator::Command::Digit(1),
        crate::calculator::Command::Digit(2),
    ] {
        c.engine.press(k);
    }
    c.engine
        .press(crate::calculator::Command::Op(crate::calculator::Op::Add));
    c.engine.press(crate::calculator::Command::Digit(7));
    let seven = c.key_rect(8);
    for (dark, name) in [(false, "calculator-light"), (true, "calculator-dark")] {
        bmp(
            &std::format!("{dir}/{name}.bmp"),
            322,
            500,
            &paint(&c, dark, Some((seven.x + 10, seven.y + 10))),
        );
    }
    c.engine.press(crate::calculator::Command::Equals);
    door_app_ui::Application::configure(&mut c, 1200, 940);
    bmp(
        &std::format!("{dir}/calculator-wide-dark.bmp"),
        1200,
        940,
        &paint(&c, true, None),
    );
}

fn paint_clock(
    c: &crate::clock::Clock,
    dark: bool,
    hover: Option<(i32, i32)>,
) -> alloc::vec::Vec<u8> {
    let fonts = FontAtlas::windows_reference_scaled(1)
        .unwrap()
        .with_unicode(
            door_workspace::assets::text_system().unwrap(),
            door_graphics_text::Family::Ui,
        );
    let (w, h) = (c.width, c.height);
    let mut px = alloc::vec![0; (w * h * 4) as usize];
    let images = ImageCache::default();
    let mut v = ui::View::new_scaled(&mut px, w, h, &fonts, 1).with_images(&images);
    v.palette.dark = dark;
    v.hover = hover;
    c.paint(&mut v);
    px
}

#[test]
fn clock_pages_run_on_real_time() {
    use crate::clock::{Clock, Page};
    let mut c = Clock::new(0);
    door_app_ui::Application::configure(&mut c, 1000, 680);
    // Navigation rows switch pages.
    let target = |c: &Clock, page: Page| {
        let i = Page::NAV.iter().position(|p| *p == page).unwrap();
        c.hits()
            .into_iter()
            .find(|(_, t)| *t == 1 + i as u32)
            .unwrap()
    };
    let (_, t) = target(&c, Page::Stopwatch);
    assert!(c.activate(t));
    assert_eq!(c.page, Page::Stopwatch);
    // The stopwatch runs on the tick's milliseconds.
    c.advance(1_000, None);
    let toggle = c.hits().into_iter().find(|(_, t)| *t == 40).unwrap().1;
    c.activate(toggle);
    c.advance(4_250, None);
    assert_eq!(
        crate::clock::stopwatch_text(c.stopwatch.elapsed(c.now)),
        "00:00:03.25"
    );
    // A started timer posts one toast when it finishes.
    c.page = Page::Timer;
    let start = c.hits().into_iter().find(|(_, t)| *t == 100).unwrap().1;
    c.activate(start);
    c.advance(4_250 + 60_000, None);
    c.advance(4_250 + 61_000, None);
    assert_eq!(c.toasts.len(), 1);
    assert_eq!(c.toasts[0].1, "1 min is done");
}

/// Review renders (`DUMP_DIR=<dir> cargo test -p door-inbox render_clock --
/// --ignored`).
#[test]
#[ignore]
fn render_clock() {
    use crate::clock::{Clock, Page};
    let dir = std::env::var("DUMP_DIR").unwrap();
    let bmp = |name: &str, c: &Clock, dark: bool| {
        let px = paint_clock(c, dark, None);
        let (w, h) = (c.width, c.height);
        let mut out = std::vec::Vec::new();
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(54 + w * h * 4).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(-(h as i32)).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        out.extend_from_slice(&px);
        std::fs::write(std::format!("{dir}/{name}.bmp"), out).unwrap();
    };
    let mut c = Clock::new(0);
    door_app_ui::Application::configure(&mut c, 1000, 680);
    c.advance(10_000, None);
    c.timers[1].toggle(10_000);
    c.advance(70_000, None);
    for (page, name) in [
        (Page::Timer, "clock-timer"),
        (Page::Alarm, "clock-alarm"),
        (Page::Stopwatch, "clock-stopwatch"),
        (Page::Focus, "clock-focus"),
        (Page::WorldClock, "clock-world"),
    ] {
        c.page = page;
        bmp(name, &c, true);
    }
    c.page = Page::Timer;
    bmp("clock-timer-light", &c, false);
}

fn paint_paint(
    p: &crate::paint::Paint,
    dark: bool,
    hover: Option<(i32, i32)>,
) -> alloc::vec::Vec<u8> {
    let fonts = FontAtlas::windows_reference_scaled(1)
        .unwrap()
        .with_unicode(
            door_workspace::assets::text_system().unwrap(),
            door_graphics_text::Family::Ui,
        );
    let (w, h) = (p.width, p.height);
    let mut px = alloc::vec![0; (w * h * 4) as usize];
    let images = ImageCache::default();
    let mut v = ui::View::new_scaled(&mut px, w, h, &fonts, 1).with_images(&images);
    v.palette.dark = dark;
    v.hover = hover;
    p.paint(&mut v);
    px
}
fn bmp(dir: &str, name: &str, w: u32, h: u32, px: &[u8]) {
    let mut out = std::vec::Vec::new();
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + w * h * 4).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(px);
    std::fs::write(std::format!("{dir}/{name}.bmp"), out).unwrap();
}

/// Paint draws with real tools: a brush stroke, a filled shape, a fill,
/// a 3D shape and text land on the canvas, and undo restores it.
#[test]
fn paint_tools_change_the_canvas_and_undo_restores_it() {
    use crate::paint::{Paint, Tool};
    use door_app_ui::Application;
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let mut p = Paint::new(1001);
    p.configure(1200, 660);
    let white = p.doc.pixel(10, 10).unwrap();
    let (x0, y0) = p.to_logical(100.0, 100.0);
    let (x1, y1) = p.to_logical(300.0, 120.0);
    p.tool = Tool::Brush;
    assert!(p.pointer_down(x0, y0, &fonts));
    p.pointer_motion(x1, y1, &fonts);
    p.pointer_up();
    let mid = p.doc.pixel(200, 110).unwrap();
    assert!(mid[0] < 64, "the stroke is dark: {mid:?}");
    assert!(p.history.can_undo());
    p.undo();
    assert_eq!(p.doc.pixel(200, 110).unwrap(), white);
    p.redo();
    // A filled rectangle.
    p.tool = Tool::Shape;
    p.shape = crate::paint::raster::Shape::Rectangle;
    p.fill_solid = true;
    p.secondary = crate::paint::raster::Rgba::rgb(0, 0, 255);
    let (a, b) = (p.to_logical(400.0, 300.0), p.to_logical(600.0, 400.0));
    p.pointer_down(a.0, a.1, &fonts);
    p.pointer_motion(b.0, b.1, &fonts);
    p.pointer_up();
    assert_eq!(p.doc.pixel(500, 350).unwrap(), [255, 0, 0, 255]);
    // Fill the background red.
    p.tool = Tool::Fill;
    p.primary = crate::paint::raster::Rgba::rgb(255, 0, 0);
    let c = p.to_logical(1000.0, 200.0);
    p.pointer_down(c.0, c.1, &fonts);
    p.pointer_up();
    assert_eq!(p.doc.pixel(1100, 50).unwrap(), [0, 0, 255, 255]);
    assert_eq!(
        p.doc.pixel(500, 350).unwrap(),
        [255, 0, 0, 255],
        "the shape keeps its fill"
    );
    // Select, copy, paste, move.
    p.select_all();
    p.copy();
    p.paste();
    assert!(p.selection.as_ref().unwrap().floating.is_some());
    p.commit();
    assert!(p.dirty);
}

/// Writes Paint's window to $DUMP_DIR (`cargo test -p door-inbox render_paint -- --ignored`).
#[test]
#[ignore]
fn render_paint() {
    use crate::paint::raster::{Rgba, Shape, Solid};
    use crate::paint::{Menu, Paint, Tool};
    use door_app_ui::Application;
    let dir = std::env::var("DUMP_DIR").unwrap();
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let mut p = Paint::new(1001);
    p.configure(1200, 660);
    p.display(1200, 660, 100);
    // A little drawing.
    let stroke = |p: &mut Paint, pts: &[(f32, f32)]| {
        let s = p.to_logical(pts[0].0, pts[0].1);
        p.pointer_down(s.0, s.1, &fonts);
        for q in &pts[1..] {
            let l = p.to_logical(q.0, q.1);
            p.pointer_motion(l.0, l.1, &fonts);
        }
        p.pointer_up();
    };
    p.tool = Tool::Fill;
    p.primary = Rgba::rgb(153, 217, 234);
    stroke(&mut p, &[(40.0, 40.0)]);
    p.tool = Tool::Shape;
    p.shape = Shape::Oval;
    p.fill_solid = true;
    p.secondary = Rgba::rgb(255, 201, 14);
    p.primary = Rgba::rgb(255, 127, 39);
    p.stroke = 5;
    stroke(&mut p, &[(820.0, 60.0), (960.0, 200.0)]);
    p.tool = Tool::Brush;
    p.size_brush = 14;
    p.primary = Rgba::rgb(34, 177, 76);
    stroke(
        &mut p,
        &[
            (0.0, 560.0),
            (200.0, 520.0),
            (420.0, 540.0),
            (640.0, 500.0),
            (900.0, 530.0),
            (1152.0, 510.0),
        ],
    );
    p.tool = Tool::Fill;
    stroke(&mut p, &[(600.0, 620.0)]);
    p.tool = Tool::Solid;
    p.solid = Solid::Cube;
    p.primary = Rgba::rgb(237, 28, 36);
    stroke(&mut p, &[(160.0, 220.0), (380.0, 440.0)]);
    p.solid = Solid::Sphere;
    p.primary = Rgba::rgb(63, 72, 204);
    stroke(&mut p, &[(470.0, 300.0), (650.0, 480.0)]);
    p.tool = Tool::Shape;
    p.shape = Shape::Star5;
    p.fill_solid = true;
    p.secondary = Rgba::rgb(255, 242, 0);
    p.primary = Rgba::rgb(0, 0, 0);
    p.stroke = 3;
    stroke(&mut p, &[(700.0, 300.0), (860.0, 450.0)]);
    p.tool = Tool::Brush;
    p.primary = Rgba::rgb(0, 0, 0);
    p.size_brush = 8;
    p.fill_solid = false;
    for dark in [false, true] {
        let name = if dark { "paint-dark" } else { "paint-light" };
        bmp(
            &dir,
            name,
            p.width,
            p.height,
            &paint_paint(&p, dark, Some((400, 300))),
        );
    }
    for (menu, name) in [
        (Menu::File, "paint-file"),
        (Menu::Brushes, "paint-brushes"),
        (Menu::Colors, "paint-colors"),
        (Menu::Solids, "paint-3d"),
    ] {
        p.menu = Some(menu);
        bmp(&dir, name, p.width, p.height, &paint_paint(&p, false, None));
    }
    p.menu = None;
    p.layers_open = true;
    p.add_layer();
    bmp(
        &dir,
        "paint-layers-dark",
        p.width,
        p.height,
        &paint_paint(&p, true, None),
    );
    p.layers_open = false;
    // The guest's window: 896 px wide (70% of 1280), groups collapsed.
    let mut n = Paint::new(1001);
    n.configure(896, 590);
    n.display(896, 590, 100);
    bmp(
        &dir,
        "paint-narrow-light",
        n.width,
        n.height,
        &paint_paint(&n, false, None),
    );
    bmp(
        &dir,
        "paint-narrow-dark",
        n.width,
        n.height,
        &paint_paint(&n, true, None),
    );
    n.menu = Some(Menu::Group(crate::paint::view::SHAPES));
    bmp(
        &dir,
        "paint-narrow-shapes",
        n.width,
        n.height,
        &paint_paint(&n, false, None),
    );
    n.menu = Some(Menu::Group(crate::paint::view::TOOLS));
    bmp(
        &dir,
        "paint-narrow-tools",
        n.width,
        n.height,
        &paint_paint(&n, true, None),
    );
    n.menu = None;
    n.zoom = 2000;
    bmp(
        &dir,
        "paint-narrow-zoomed",
        n.width,
        n.height,
        &paint_paint(&n, false, None),
    );
    p.dialog = Some(crate::paint::Dialog::Resize {
        pixels: false,
        h: "100".into(),
        v: "100".into(),
        lock: true,
        field: 0,
    });
    bmp(
        &dir,
        "paint-resize",
        p.width,
        p.height,
        &paint_paint(&p, false, None),
    );
}

fn paint_snipping(p: &crate::snipping::Snipping, dark: bool) -> alloc::vec::Vec<u8> {
    let fonts = FontAtlas::windows_reference_scaled(1)
        .unwrap()
        .with_unicode(
            door_workspace::assets::text_system().unwrap(),
            door_graphics_text::Family::Ui,
        );
    let (w, h) = (p.width, p.height);
    let mut px = alloc::vec![0; (w * h * 4) as usize];
    let images = ImageCache::default();
    let mut v = ui::View::new_scaled(&mut px, w, h, &fonts, 1).with_images(&images);
    v.palette.dark = dark;
    p.paint(&mut v);
    px
}

/// Snipping Tool shows a snip and marks it up with the pen.
#[test]
fn snipping_marks_up_a_snip() {
    use crate::snipping::Snipping;
    use door_app_ui::Application;
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let mut s = Snipping::new(1001);
    s.configure(900, 560);
    s.open_snip(400, 300, alloc::vec![255; 400 * 300 * 4]);
    let a = {
        let c = s.card();
        (c.x + c.w as i32 / 2 - 40, c.y + c.h as i32 / 2)
    };
    assert!(s.pointer_down(a.0, a.1, &fonts));
    s.pointer_motion(a.0 + 80, a.1, &fonts);
    s.pointer_up();
    let (ix, iy) = s.to_image(a.0 + 40, a.1).unwrap();
    let d = s.image.as_ref().unwrap();
    assert!(
        d.pixel(ix as i32, iy as i32).unwrap()[0] < 100,
        "ink on the snip"
    );
}

/// Writes Snipping Tool to $DUMP_DIR (`cargo test -p door-inbox render_snipping -- --ignored`).
#[test]
#[ignore]
fn render_snipping() {
    use crate::snipping::{Menu, Snipping};
    use door_app_ui::Application;
    let dir = std::env::var("DUMP_DIR").unwrap();
    let mut s = Snipping::new(1001);
    s.configure(515, 237);
    for dark in [false, true] {
        bmp(
            &dir,
            if dark {
                "snip-empty-dark"
            } else {
                "snip-empty-light"
            },
            s.width,
            s.height,
            &paint_snipping(&s, dark),
        );
    }
    // A snip: the Paint render, marked up.
    let mut p = crate::paint::Paint::new(1001);
    p.configure(1000, 600);
    p.display(1000, 600, 100);
    let shot = paint_paint(&p, true, None);
    s.configure(1100, 640);
    s.open_snip(1000, 600, shot);
    bmp(
        &dir,
        "snip-open-dark",
        s.width,
        s.height,
        &paint_snipping(&s, true),
    );
    s.menu = Some(Menu::Mode);
    bmp(
        &dir,
        "snip-mode-light",
        s.width,
        s.height,
        &paint_snipping(&s, false),
    );
}

/// Writes Weather to $DUMP_DIR (`cargo test -p door-inbox render_weather -- --ignored`).
#[test]
#[ignore]
fn render_weather() {
    use door_app_ui::Application;
    let dir = std::env::var("DUMP_DIR").unwrap();
    let mut w = crate::weather::Weather::new();
    w.configure(1210, 900);
    for dark in [false, true] {
        let fonts = FontAtlas::windows_reference_scaled(1)
            .unwrap()
            .with_unicode(
                door_workspace::assets::text_system().unwrap(),
                door_graphics_text::Family::Ui,
            );
        let mut px = alloc::vec![0; (w.width * w.height * 4) as usize];
        let images = ImageCache::default();
        {
            let mut v =
                ui::View::new_scaled(&mut px, w.width, w.height, &fonts, 1).with_images(&images);
            v.palette.dark = dark;
            w.paint(&mut v);
        }
        bmp(
            &dir,
            if dark {
                "weather-dark"
            } else {
                "weather-light"
            },
            w.width,
            w.height,
            &px,
        );
    }
}
