//! Exercise the actual native Terminal painter with the pinned density atlases.
#![allow(dead_code)]
extern crate alloc;
use door_graphics_text::FontAtlas;
use door_workspace::terminal_model::{Layout, Page, Profile};
// The library's own View, so Terminal shares the caption-tab painter with
// the workspace binary exactly as it runs natively.
use door_workspace::ui;
#[path = "../src/terminal.rs"]
mod terminal;
fn pixel(bytes: &[u8], width: u32, x: u32, y: u32) -> &[u8] {
    let at = ((y * width + x) * 4) as usize;
    &bytes[at..at + 4]
}
#[test]
fn render_terminal_review_pages_from_actual_native_painter() {
    let output =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/terminal-review");
    std::fs::create_dir_all(&output).unwrap();
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let mono = FontAtlas::monospace_reference_scaled(1).unwrap();
    let (width, height) = (1280, 800);
    let (_, rgba) = qoi::decode_to_vec(include_bytes!(
        "../../../assets/wallpapers/windows/bloom.qoi"
    ))
    .unwrap();
    let mut wallpaper = vec![255; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            for c in 0..3 {
                let mut sum = 0u32;
                for dy in 0..3 {
                    for dx in 0..3 {
                        sum += u32::from(rgba[((y * 3 + dy) * 3840 + x * 3 + dx) * 4 + 2 - c]);
                    }
                }
                wallpaper[(y * width + x) * 4 + c] = ((sum + 4) / 9) as u8;
            }
        }
    }
    let mut t = terminal::Terminal::for_home("/users/1001".into());
    t.configure(1128, 632);
    t.state.new_tab(Profile::Cmd);
    t.state.new_tab(Profile::PowerShell);
    t.state.new_tab(Profile::Dsl);
    t.state.active = 0;
    t.recent = vec![
        "/users/1001/Documents/projects".into(),
        "/compat/Windows/System32".into(),
    ];
    for dark in [false, true] {
        for (name, page) in [
            ("console", Page::Console),
            ("home", Page::Home),
            ("settings", Page::Settings),
            ("about", Page::About),
            ("profiles", Page::Profiles),
        ] {
            t.state.page = page;
            let mut pixels = vec![0; 1128 * 632 * 4];
            let mut view =
                ui::View::new_scaled(&mut pixels, 1128, 632, &fonts, 1).with_monospace(Some(&mono));
            view.palette.dark = dark;
            t.draw(&mut view);
            assert!(
                pixels
                    .chunks_exact(4)
                    .all(|p| p[..3].iter().all(|c| *c <= p[3]))
            );
            use door_graphics_core::*;
            let owner = Owner {
                pid: 1,
                session: 1,
                epoch: 1,
            };
            let mut engine = Engine::new(
                width as u32,
                height as u32,
                if dark { Theme::Dark } else { Theme::Light },
                Limits::default(),
            )
            .unwrap();
            engine.set_wallpaper_material(&wallpaper).unwrap();
            let (side_margin, max_width) = t.caption_region();
            let window = engine
                .create_client_caption(
                    owner,
                    Rect::new(52, 92, 1128, 632).unwrap(),
                    CaptionLayout {
                        tabbed: true,
                        height: door_workspace::caption_tabs::HEIGHT,
                        max_width,
                        side_margin,
                    },
                )
                .unwrap();
            let epoch = engine.window_info(window).unwrap().epoch;
            engine.acknowledge_configure(owner, window, epoch).unwrap();
            let buffer = engine
                .register_buffer(
                    owner,
                    SurfaceDescriptor::new(1128, 632, 1128 * 4, 1128 * 632 * 4).unwrap(),
                )
                .unwrap();
            engine.attach(owner, window, buffer, epoch).unwrap();
            engine.commit(owner, window, pixels).unwrap();
            let frame = engine
                .render_with_background(
                    |canvas| {
                        canvas.restore_bgra(&wallpaper).unwrap();
                    },
                    |_, _| {},
                )
                .unwrap();
            let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
            for p in frame.pixels.chunks_exact(4) {
                ppm.extend_from_slice(&[p[2], p[1], p[0]]);
            }
            let suffix = if dark { "-dark" } else { "" };
            std::fs::write(output.join(format!("{name}{suffix}.ppm")), ppm).unwrap();
        }
    }
}
#[test]
fn actual_terminal_pages_profiles_and_fixed_cells_paint_at_every_native_density() {
    for scale in 1..=3 {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let mono = FontAtlas::monospace_reference_scaled(scale).unwrap();
        for (style, cell) in [
            (door_graphics_text::TextStyle::Regular12, 8),
            (door_graphics_text::TextStyle::Regular14, 9),
            (door_graphics_text::TextStyle::Regular20, 12),
        ] {
            for ch in ' '..='~' {
                assert!(mono.measure(&ch.to_string(), style).unwrap().advance_width <= cell);
            }
        }
        for width in [520, 1050] {
            let height = 650;
            let mut terminal = terminal::Terminal::for_home("/users/1001".into());
            terminal.configure(width, height);
            terminal.state.tab().output(
                "Unicode: 文\n\x1b bad-control\nA long row is wrapped to the actual console width.",
            );
            for dark in [false, true] {
                for page in [Page::Console, Page::Settings, Page::About, Page::Profiles] {
                    terminal.state.page = page;
                    let mut pixels = vec![0; (width * height * scale * scale * 4) as usize];
                    let mut view = ui::View::new_scaled(
                        &mut pixels,
                        width * scale,
                        height * scale,
                        &fonts,
                        scale,
                    );
                    view.palette.dark = dark;
                    view = view.with_monospace(Some(&mono));
                    terminal.draw(&mut view);
                    assert_eq!(
                        view.palette.dark, dark,
                        "Terminal must preserve the caller theme"
                    );
                    assert!(
                        pixels
                            .chunks_exact(4)
                            .all(|p| p[..3].iter().all(|c| *c <= p[3]))
                    );
                    if page == Page::Console {
                        let rail = Layout {
                            width,
                            height,
                            rail_hidden: false,
                        }
                        .rail();
                        assert_eq!(
                            pixel(&pixels, width * scale, (rail + 2) * scale, 100 * scale),
                            // Campbell #0C0C0C at the default 85 % opacity.
                            &[10, 10, 10, 216]
                        );
                    } else {
                        let rail = Layout {
                            width,
                            height,
                            rail_hidden: false,
                        }
                        .rail();
                        assert_eq!(
                            pixel(&pixels, width * scale, (rail + 10) * scale, 110 * scale),
                            // Windows 11 Settings base: Mica light, #202020 dark.
                            if dark {
                                &[32, 32, 32, 255]
                            } else {
                                &[250, 247, 244, 255]
                            },
                            "Information pages must follow the application theme"
                        );
                        let mut contrasting_ink = 0;
                        for y in 70 * scale..102 * scale {
                            for x in (rail + 28) * scale..(rail + 180) * scale {
                                let p = pixel(&pixels, width * scale, x, y);
                                if if dark {
                                    p[..3].iter().all(|c| *c > 180)
                                } else {
                                    p[..3].iter().all(|c| *c < 100)
                                } {
                                    contrasting_ink += 1;
                                }
                            }
                        }
                        assert!(
                            contrasting_ink > 30 * scale * scale,
                            "Heading needs readable theme contrast"
                        );
                    }
                }
            }
            terminal.state.new_tab(Profile::PowerShell);
            let mut pixels = vec![0; (width * height * scale * scale * 4) as usize];
            let mut view =
                ui::View::new_scaled(&mut pixels, width * scale, height * scale, &fonts, scale);
            terminal.draw(&mut view);
            // Runtime requirements are drawn by the actual app, with no fabricated
            // process output or sample PowerShell prompt in its session model.
            assert!(terminal.state.tab().lines.is_empty());
            assert!(!terminal.state.tab().profile.available());
        }
    }
}
#[test]
fn terminal_pointer_targets_match_the_drawn_controls_and_tab_closes() {
    let mut t = terminal::Terminal::for_home("/users/1001".into());
    t.configure(1050, 650);
    assert_eq!(t.target(100, 84), Some(1));
    assert_eq!(t.target(100, 384), Some(2));
    assert_eq!(t.target(100, 424), Some(3));
    // Profiles: Doorsh, Command Prompt, PowerShell, DSL, then Add profile.
    for (y, id) in [(166, 10), (206, 11), (246, 12), (286, 13), (326, 20)] {
        assert_eq!(t.target(100, y), Some(id));
    }
    // The console toolbar: Full screen, Find, the pane button, Clear, More.
    for (x, id) in [
        (1050 - 186, 38),
        (1050 - 146, 31),
        (1050 - 106, 36),
        (1050 - 66, 32),
        (1050 - 26, 37),
    ] {
        assert_eq!(t.target(x, 80), Some(id), "x={x}");
    }
    t.state.page = Page::Settings;
    for (y, id) in [(150, 40), (240, 41), (330, 42)] {
        assert_eq!(t.target(300, y), Some(id));
    }
    assert_eq!(t.target(960, 620), Some(43));
    assert_eq!(t.target(300, 412), Some(44));
    assert_eq!(t.target(300, 429), Some(44));
    assert_eq!(t.target(300, 385), None);
    assert_eq!(t.target(300, 465), None);
    t.state.page = Page::Console;
    let layout = Layout {
        width: 1050,
        height: 650,
        rail_hidden: false,
    };
    let strip = layout.strip(1, 0);
    let tab = strip.tab(0).unwrap();
    let close = strip.close(0).unwrap();
    // Explorer's caption strip: the first tab starts at the left edge.
    assert_eq!((tab.x, tab.y), (8, 3));
    assert_eq!(t.target(tab.x + 20, 20), Some(terminal::TAB));
    assert_eq!(
        t.target(close.x + 6, close.y + 6),
        Some(terminal::TAB_CLOSE)
    );
    let plus = strip.plus().unwrap();
    assert_eq!(t.target(plus.x + 16, plus.y + 16), Some(30));
    // Windows Terminal's "+" / drop-down split button (64 x 24 at y 8).
    let (half, profiles) = t.new_tab_buttons().unwrap();
    let profiles = profiles.unwrap();
    assert_eq!(
        (half.x, half.y, half.width, half.height),
        (plus.x, 8, 32, 24)
    );
    assert_eq!(t.target(profiles.x + 16, 20), Some(20));
    // Empty caption right of the split button belongs to the window service.
    for x in (profiles.x + profiles.width as i32 + 1..1050).step_by(9) {
        assert_eq!(t.target(x, 20), None, "x={x}");
    }
    // The rail starts below the caption.
    assert_eq!(t.target(20, 20), Some(terminal::TAB));
    assert_eq!(t.target(20, 45), None);
    assert_eq!(t.target(-1, 20), None);
}
#[test]
fn wheel_scrolls_scrollback_only_over_the_console() {
    let mut t = terminal::Terminal::for_home("/users/1001".into());
    t.configure(1050, 650);
    t.state.tab().lines.clear();
    t.state
        .tab()
        .output(&(0..200).map(|i| format!("row {i}\n")).collect::<String>());
    assert!(t.wheel(600, 300, 1));
    assert_eq!(t.state.tabs[0].scroll, 3);
    assert!(t.wheel(600, 300, 2));
    assert_eq!(t.state.tabs[0].scroll, 9);
    assert!(t.wheel(600, 300, -1));
    assert_eq!(t.state.tabs[0].scroll, 6);
    // Over the rail or caption nothing scrolls.
    assert!(!t.wheel(100, 300, 1));
    assert!(!t.wheel(600, 20, 1));
    while t.wheel(600, 300, -5) {}
    assert_eq!(t.state.tabs[0].scroll, 0);
    t.state.page = Page::Settings;
    assert!(!t.wheel(600, 300, 1));
}
#[test]
fn acrylic_background_is_translucent_until_transparency_effects_are_off() {
    use door_service_protocol::desktop::Controls;
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let mono = FontAtlas::monospace_reference_scaled(1).unwrap();
    let mut t = terminal::Terminal::for_home("/users/1001".into());
    t.configure(1050, 650);
    t.state.preferences.opacity = 100;
    for reduce in [false, true] {
        t.controls_changed(Controls::default().with(Controls::REDUCE_TRANSPARENCY, reduce));
        assert_eq!(t.acrylic(), !reduce);
        for dark in [false, true] {
            t.state.preferences.opacity = 85;
            let mut pixels = vec![0; 1050 * 650 * 4];
            let mut view =
                ui::View::new_scaled(&mut pixels, 1050, 650, &fonts, 1).with_monospace(Some(&mono));
            view.palette.dark = dark;
            t.draw(&mut view);
            assert!(
                pixels
                    .chunks_exact(4)
                    .all(|p| p[..3].iter().all(|c| *c <= p[3]))
            );
            // Empty tab row, navigation pane background and console.
            let row = pixel(&pixels, 1050, 900, 20)[3];
            let pane = pixel(&pixels, 1050, 150, 600)[3];
            let console = pixel(&pixels, 1050, 600, 500)[3];
            if reduce {
                assert_eq!([row, pane, console], [255; 3]);
            } else {
                assert_eq!(pane, if dark { 199 } else { 204 });
                assert!(row < 255 && console < 255);
                // Rail text strokes stay (near) opaque over the tint.
                let ink = (60..200)
                    .flat_map(|x| (74..92).map(move |y| (x, y)))
                    .filter(|&(x, y)| pixel(&pixels, 1050, x, y)[3] >= 245)
                    .count();
                assert!(ink > 20, "opaque label strokes");
            }
        }
    }
}

#[test]
fn light_console_prompt_stays_readable_independently_of_application_theme() {
    for scale in 1..=3 {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let mono = FontAtlas::monospace_reference_scaled(scale).unwrap();
        for dark in [false, true] {
            let mut terminal = terminal::Terminal::for_home("/users/1001".into());
            terminal.configure(1050, 650);
            terminal.state.preferences.light_console = true;
            terminal.state.tab().lines.clear();
            let mut pixels = vec![0; (1050 * 650 * scale * scale * 4) as usize];
            let mut view =
                ui::View::new_scaled(&mut pixels, 1050 * scale, 650 * scale, &fonts, scale)
                    .with_monospace(Some(&mono));
            view.palette.dark = dark;
            terminal.draw(&mut view);
            let mut ink = 0;
            for y in 78 * scale..99 * scale {
                for x in 226 * scale..350 * scale {
                    let p = pixel(&pixels, 1050 * scale, x, y);
                    if p[..3].iter().all(|c| *c < 100) {
                        ink += 1;
                    }
                }
            }
            assert!(
                ink > 30 * scale * scale,
                "Light prompt must have dark strokes"
            );
        }
    }
}
#[test]
fn tabs_and_add_button_never_enter_the_trusted_window_controls() {
    use door_graphics_core::*;
    for scale in 1..=3 {
        for width in [320, 520, 1050, 1128] {
            for count in 1..=8 {
                let l = Layout {
                    width,
                    height: 632,
                    rail_hidden: false,
                };
                let mut t = terminal::Terminal::for_home("/".into());
                t.configure(width, 632);
                for _ in 1..count {
                    t.state.new_tab(Profile::Native);
                }
                let mut e =
                    Engine::new(width * scale, 632 * scale, Theme::Light, Limits::default())
                        .unwrap();
                e.set_scale(scale).unwrap();
                let owner = Owner {
                    pid: 1,
                    session: 1,
                    epoch: 1,
                };
                let (side_margin, max_width) = t.caption_region();
                let id = e
                    .create_client_caption(
                        owner,
                        Rect::new(0, 0, width * scale, 632 * scale).unwrap(),
                        CaptionLayout {
                            tabbed: true,
                            height: door_workspace::caption_tabs::HEIGHT,
                            side_margin: side_margin * scale,
                            max_width: max_width * scale,
                        },
                    )
                    .unwrap();
                let strip = l.strip(count, t.state.active);
                for i in strip.range() {
                    let Some(r) = strip.tab(i) else { continue };
                    let x = r.x + r.width as i32 / 2;
                    assert_eq!(
                        e.hit_test(x * scale as i32, 28 * scale as i32),
                        Some((id, HitRegion::Client))
                    );
                    assert!(matches!(
                        t.target(x, 28),
                        Some(terminal::TAB..=terminal::TAB_CLOSE_LAST)
                    ));
                }
                if let Some(p) = strip.plus() {
                    let x = p.x + p.width as i32 / 2;
                    assert!(p.x + p.width as i32 <= width as i32 - 138);
                    assert_eq!(t.target(x, 20), Some(30));
                    assert_eq!(
                        e.hit_test(x * scale as i32, 20 * scale as i32),
                        Some((id, HitRegion::Client))
                    );
                    // The split button's profile drop-down is the app's too.
                    let right = match t.new_tab_buttons() {
                        Some((_, Some(d))) => {
                            let dx = d.x + d.width as i32 / 2;
                            assert_eq!(t.target(dx, 20), Some(20));
                            assert_eq!(
                                e.hit_test(dx * scale as i32, 20 * scale as i32),
                                Some((id, HitRegion::Client))
                            );
                            d.x + d.width as i32
                        }
                        _ => p.x + p.width as i32,
                    };
                    // Beyond them the caption is the service's (drag), not the app's.
                    let beyond = right + 4;
                    if beyond < width as i32 - 140 {
                        assert_ne!(
                            e.hit_test(beyond * scale as i32, 20 * scale as i32),
                            Some((id, HitRegion::Client))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn console_opacity_is_one_premultiplied_layer_in_both_schemes_and_themes() {
    for scale in 1..=3 {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let mono = FontAtlas::monospace_reference_scaled(scale).unwrap();
        for dark in [false, true] {
            for light_console in [false, true] {
                for opacity in [50, 85, 100] {
                    let (w, h) = (660u32, 530u32);
                    let mut t = terminal::Terminal::for_home("/users/1001".into());
                    t.configure(w, h);
                    t.state.preferences.light_console = light_console;
                    t.state.preferences.opacity = opacity;
                    let mut p = vec![0; (w * h * scale * scale * 4) as usize];
                    let mut v = ui::View::new_scaled(&mut p, w * scale, h * scale, &fonts, scale)
                        .with_monospace(Some(&mono));
                    v.palette.dark = dark;
                    t.draw(&mut v);
                    let actual = pixel(&p, w * scale, 240 * scale, 240 * scale);
                    let alpha = opacity as u16 * 255 / 100;
                    // The dark scheme is Windows Terminal Campbell's #0C0C0C.
                    let rgb = if light_console {
                        [252u16, 250, 247]
                    } else {
                        [12u16, 12, 12]
                    };
                    let expected = [
                        ((rgb[0] * alpha + 127) / 255) as u8,
                        ((rgb[1] * alpha + 127) / 255) as u8,
                        ((rgb[2] * alpha + 127) / 255) as u8,
                        alpha as u8,
                    ];
                    assert_eq!(
                        actual, expected,
                        "opacity={opacity}, scheme={light_console}, theme={dark}, scale={scale}"
                    );
                    let over = |background: u16| {
                        (actual[0] as u16 + (background * (255 - alpha) + 127) / 255) as u8
                    };
                    if opacity == 100 {
                        assert_eq!(over(0), over(255));
                    } else {
                        assert!(over(255) > over(0));
                    }
                }
            }
        }
    }
}

/// New windows open at 70% x 78% of the work area (`initial_window`):
/// Terminal's 1128x632 request becomes 896x524 at 1280x720, 717x562 at
/// 1024x768 and 896x587 at 1280x800. Every page paints there, and the tab
/// strip keeps clear of the trusted caption buttons.
#[test]
fn terminal_lays_out_at_the_default_window_sizes() {
    let output_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/terminal-review");
    std::fs::create_dir_all(&output_dir).unwrap();
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let mono = FontAtlas::monospace_reference_scaled(1).unwrap();
    for output in [(1280, 720), (1024, 768), (1280, 800), (1920, 1080)] {
        let (_, _, w, h) = door_graphics_core::initial_window(output, (1128, 632), 0, None);
        let mut t = terminal::Terminal::for_home("/users/1001".into());
        t.configure(w, h);
        t.state.new_tab(Profile::PowerShell);
        t.state.active = 0;
        for (name, page) in [
            ("home", Page::Console),
            ("settings", Page::Settings),
            ("profiles", Page::Profiles),
        ] {
            t.state.page = page;
            let mut pixels = vec![0; (w * h * 4) as usize];
            {
                let mut v =
                    ui::View::new_scaled(&mut pixels, w, h, &fonts, 1).with_monospace(Some(&mono));
                t.draw(&mut v);
            }
            if (w, h) == (896, 524) || (w, h) == (717, 562) {
                let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
                ppm.extend(pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]));
                std::fs::write(output_dir.join(format!("default-{w}x{h}-{name}.ppm")), ppm)
                    .unwrap();
            }
        }
        let strip = Layout {
            width: w,
            height: h,
            rail_hidden: false,
        }
        .strip(2, 0);
        if let Some(p) = strip.plus() {
            assert!(p.x + p.width as i32 <= w as i32 - 138, "{w}x{h}");
        }
    }
}
