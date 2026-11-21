//! Contrast themes through the real shell painters: the taskbar and Start
//! paint solid system colours (no acrylic, blur or shadow) and their text
//! keeps at least 7:1 against its background, as Windows' themes do.
//! A separate test binary: `paint::set_contrast` is process-wide.
use door_abi::clock::WallClock;
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::FontAtlas;
use door_service_protocol::desktop::{ContrastTheme, Controls, SystemColors};
use door_window::{apps::App, paint, start, taskbar};

fn bgra(c: [u8; 3]) -> [u8; 4] {
    [c[2], c[1], c[0], 255]
}
fn luminance(rgb: [u8; 3]) -> f64 {
    let channel = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
}
fn ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
/// An ink colour lying between Window and WindowText (Night sky's
/// HighlightText 2B2B2B) may equally be an anti-aliased WindowText edge.
fn edge(s: SystemColors, p: [u8; 3]) -> bool {
    p != s.window_text
        && (0..3).all(|i| {
            p[i] >= s.window[i].min(s.window_text[i]) && p[i] <= s.window[i].max(s.window_text[i])
        })
}
/// Minimum contrast of text-role pixels in `r` against the most common
/// surface role around them, and how many were examined.
fn text_contrast(s: SystemColors, pixels: &[u8], width: u32, r: Rect) -> (f64, usize, (i32, i32)) {
    let at = |x: i32, y: i32| {
        let o = (y as usize * width as usize + x as usize) * 4;
        [pixels[o + 2], pixels[o + 1], pixels[o]]
    };
    let surfaces = [s.window, s.highlight];
    let inks = [s.window_text, s.highlight_text];
    let (mut min, mut examined, mut worst) = (f64::MAX, 0, (0, 0));
    for y in r.y() + 6..r.bottom() - 6 {
        for x in r.x() + 6..r.right() - 6 {
            let p = at(x, y);
            if !inks.contains(&p) || surfaces.contains(&p) || edge(s, p) {
                continue;
            }
            // Skip strokes (borders, dividers): glyph strokes are short.
            let run = |dx: i32, dy: i32| {
                (1..12)
                    .take_while(|k| at(x + dx * k, y + dy * k) == p)
                    .count()
            };
            if run(1, 0) + run(-1, 0) >= 10 || run(0, 1) + run(0, -1) >= 10 {
                continue;
            }
            let mut counts = [0usize; 2];
            for v in y - 6..y + 7 {
                for u in x - 6..x + 7 {
                    if let Some(i) = surfaces.iter().position(|c| *c == at(u, v)) {
                        counts[i] += 1;
                    }
                }
            }
            let i = usize::from(counts[1] > counts[0]);
            if counts[i] == 0 {
                continue;
            }
            examined += 1;
            let q = ratio(p, surfaces[i]);
            if q < min {
                (min, worst) = (q, (x, y));
            }
        }
    }
    (min, examined, worst)
}
fn pixel(pixels: &[u8], width: u32, x: i32, y: i32) -> [u8; 4] {
    let o = (y as usize * width as usize + x as usize) * 4;
    pixels[o..o + 4].try_into().unwrap()
}
fn save(name: &str, pixels: &[u8], width: u32, height: u32) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shell-review");
    std::fs::create_dir_all(&dir).unwrap();
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    ppm.extend(pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]));
    std::fs::write(dir.join(format!("{name}.ppm")), ppm).unwrap();
}

#[test]
fn contrast_themes_paint_the_taskbar_and_start_solid_with_seven_to_one_text() {
    let fonts = FontAtlas::windows_reference().unwrap();
    let (w, h) = (1280u32, 720u32);
    for theme in ContrastTheme::ALL {
        paint::set_contrast(Some(theme));
        let s = theme.colors();
        let window = bgra(s.window);
        // The desktop is the solid Window colour (the shell's wallpaper).
        let mut pixels: Vec<u8> = window.repeat((w * h) as usize);
        let layout = taskbar::Layout::with_controls(w, h, Controls::DEFAULT);
        let mut st = start::Start::default();
        st.open();
        let g = st.geometry(layout);
        let buttons = [
            taskbar::Button {
                app: App::Files,
                windows: 1,
                active: true,
            },
            taskbar::Button {
                app: App::Notes,
                windows: 1,
                active: false,
            },
            taskbar::Button {
                app: App::Settings,
                windows: 0,
                active: false,
            },
        ];
        {
            let mut c = Canvas::new(&mut pixels, w, h, w as usize * 4).unwrap();
            // Contrast themes turn transparency off: no material.
            taskbar::draw(
                &mut c,
                &fonts,
                layout,
                &taskbar::View {
                    buttons: &buttons,
                    pointer: (0, 0),
                    pressed: None,
                    press: None,
                    hover: [(None, 0); 2],
                    start_open: true,
                    search_open: false,
                    task_view_open: false,
                    quick_open: false,
                    calendar_open: false,
                    hidden_icons: None,
                    time: Some(WallClock {
                        year: 2026,
                        month: 10,
                        day: 22,
                        hour: 20,
                        minute: 20,
                        second: 0,
                    }),
                    region: 0,
                    overrides: 0,
                    network: taskbar::Net::Connected,
                    vpn: false,
                    audio: None,
                    dark: false,
                    material: None,
                },
            );
            start::draw(
                &mut c,
                &fonts,
                &st,
                layout,
                &start::Frame {
                    pointer: (0, 0),
                    dark: false,
                    slide: 0,
                    transparency: false,
                    taskbar_pins: &App::ALL,
                },
            );
        }
        let name = format!("contrast-shell-{theme:?}").to_lowercase();
        save(&name, &pixels, w, h);
        // Solid surfaces: the taskbar between its buttons and the Start
        // panel's padding are Window; no shadow falls outside the panel.
        let bar = layout.bar();
        assert_eq!(
            pixel(&pixels, w, bar.x() + 4, bar.bottom() - 4),
            window,
            "{name}: taskbar"
        );
        let panel = g.panel;
        assert_eq!(
            pixel(
                &pixels,
                w,
                panel.x() + 6,
                panel.y() + panel.height() as i32 / 2
            ),
            window,
            "{name}: Start"
        );
        for (x, y) in [
            (panel.x() - 4, panel.y() + panel.height() as i32 / 2),
            (panel.right() + 3, panel.y() + panel.height() as i32 / 2),
            (panel.x() + panel.width() as i32 / 2, panel.y() - 4),
        ] {
            assert_eq!(
                pixel(&pixels, w, x, y),
                window,
                "{name}: no shadow at ({x}, {y})"
            );
        }
        // The panel is outlined in WindowText.
        assert_eq!(
            pixel(&pixels, w, panel.x(), panel.y() + panel.height() as i32 / 2),
            bgra(s.window_text),
            "{name}: Start border"
        );
        for (part, r) in [("Start", panel), ("taskbar", bar)] {
            let (min, examined, worst) = text_contrast(s, &pixels, w, r);
            assert!(examined > 40, "{name} {part}: text found ({examined})");
            // Desert's selection pair is 6.9988 by Windows' own colours.
            assert!(
                min >= 6.995,
                "{name} {part}: text contrast {min:.3} at {worst:?}"
            );
        }
    }
    paint::set_contrast(None);
}
