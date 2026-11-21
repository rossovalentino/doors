//! Host-rendered shell captures (taskbar, Start, flyouts, menus, snap bar)
//! through the same painters the window server uses, written as PPM files to
//! `target/shell-review/` for comparison with the Windows 11 references.
use door_abi::clock::WallClock;
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::FontAtlas;
use door_service_protocol::desktop::Controls;
use door_window::{
    apps::App,
    menu::{self, Anchor},
    menus, snap, start, taskbar, tray,
};
fn directory() -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shell-review");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn wallpaper(width: u32, height: u32) -> Vec<u8> {
    let bytes = include_bytes!("../../../assets/wallpapers/windows/bloom.qoi");
    let (header, pixels) = qoi::decode_to_vec(bytes).unwrap();
    let (sw, sh) = (header.width as usize, header.height as usize);
    let channels = pixels.len() / (sw * sh);
    let mut out = vec![255u8; (width * height * 4) as usize];
    for y in 0..height as usize {
        for x in 0..width as usize {
            // Cover-fit nearest sampling.
            let sx = x * sw / width as usize;
            let sy = (y * sh / height as usize).min(sh - 1);
            let at = (sy * sw + sx) * channels;
            let o = (y * width as usize + x) * 4;
            out[o] = pixels[at + 2];
            out[o + 1] = pixels[at + 1];
            out[o + 2] = pixels[at];
        }
    }
    out
}
fn save(name: &str, pixels: &[u8], width: u32, height: u32, crop: Option<Rect>) {
    let r = crop.unwrap_or(Rect::new(0, 0, width, height).unwrap());
    let mut ppm = format!("P6\n{} {}\n255\n", r.width(), r.height()).into_bytes();
    for y in r.y() as usize..r.bottom() as usize {
        for x in r.x() as usize..r.right() as usize {
            let p = &pixels[(y * width as usize + x) * 4..][..4];
            ppm.extend([p[2], p[1], p[0]]);
        }
    }
    std::fs::write(directory().join(format!("{name}.ppm")), ppm).unwrap();
}
fn time() -> WallClock {
    WallClock {
        year: 2026,
        month: 10,
        day: 22,
        hour: 20,
        minute: 20,
        second: 0,
    }
}
struct Scene {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    layout: taskbar::Layout,
}
impl Scene {
    fn new(width: u32, height: u32, controls: Controls) -> Self {
        Self {
            width,
            height,
            pixels: wallpaper(width, height),
            layout: taskbar::Layout::with_controls(width, height, controls),
        }
    }
    fn canvas(&mut self) -> Canvas<'_> {
        Canvas::new(
            &mut self.pixels,
            self.width,
            self.height,
            self.width as usize * 4,
        )
        .unwrap()
    }
    fn bar(
        &mut self,
        fonts: &FontAtlas,
        dark: bool,
        pointer: (i32, i32),
        start_open: bool,
        quick: bool,
        calendar: bool,
    ) {
        let mut material = door_window::material::Taskbar::new(self.width, self.height).unwrap();
        assert!(material.update(&self.pixels, dark));
        let pixels = material.pixels().to_vec();
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
            taskbar::Button {
                app: App::Terminal,
                windows: 2,
                active: false,
            },
            taskbar::Button {
                app: App::Preview,
                windows: 0,
                active: false,
            },
            taskbar::Button {
                app: App::Monitor,
                windows: 0,
                active: false,
            },
        ];
        let layout = self.layout;
        let mut c = self.canvas();
        taskbar::draw(
            &mut c,
            fonts,
            layout,
            &taskbar::View {
                buttons: &buttons,
                pointer,
                pressed: None,
                press: None,
                hover: [(None, 0); 2],
                start_open,
                search_open: false,
                task_view_open: false,
                quick_open: quick,
                calendar_open: calendar,
                hidden_icons: None,
                time: Some(time()),
                region: 0,
                overrides: 0,
                network: taskbar::Net::Connected,
                vpn: false,
                audio: None,
                dark,
                material: Some(&pixels),
            },
        );
    }
}
#[test]
fn render_shell_review_captures() {
    let fonts = FontAtlas::windows_reference().unwrap();
    for dark in [true, false] {
        let theme = if dark { "dark" } else { "light" };
        // Taskbar alone, pointer over the Notes button.
        let mut s = Scene::new(1920, 1080, Controls::DEFAULT);
        let notes = s.layout.slot(taskbar::Slot::App(1), 6);
        s.bar(
            &fonts,
            dark,
            (notes.x() + 10, notes.y() + 10),
            false,
            false,
            false,
        );
        save(
            &format!("taskbar-{theme}"),
            &s.pixels,
            1920,
            1080,
            Some(Rect::new(0, 1000, 1920, 80).unwrap()),
        );
        let mut s = Scene::new(
            1920,
            1080,
            Controls::DEFAULT.with(Controls::SEARCH_ICON_ONLY, true),
        );
        s.bar(&fonts, dark, (0, 0), false, false, false);
        save(
            &format!("taskbar-icon-search-{theme}"),
            &s.pixels,
            1920,
            1080,
            Some(Rect::new(560, 1020, 800, 60).unwrap()),
        );
        // Start (Category view) over the wallpaper.
        for (w, h, view, name) in [
            (1920, 1080, start::View::Category, "start"),
            (1280, 720, start::View::List, "start-list"),
            (1280, 720, start::View::Grid, "start-grid"),
        ] {
            let mut s = Scene::new(w, h, Controls::DEFAULT);
            let mut st = start::Start {
                view,
                ..start::Start::default()
            };
            st.open();
            let g = st.geometry(s.layout);
            let scene = s.pixels.clone();
            assert!(st.backdrop.build(&scene, w as usize * 4, g.panel, 8));
            let tile = g.pinned_tile(1, 0).unwrap();
            s.bar(&fonts, dark, (0, 0), true, false, false);
            let layout = s.layout;
            let mut c = s.canvas();
            start::draw(
                &mut c,
                &fonts,
                &st,
                layout,
                &start::Frame {
                    pointer: (tile.x() + 20, tile.y() + 20),
                    dark,
                    slide: 0,
                    transparency: true,
                    taskbar_pins: &App::ALL,
                },
            );
            save(&format!("{name}-{theme}"), &s.pixels, w, h, None);
            let crop = paint_crop(g.panel, w, h);
            save(
                &format!("{name}-{theme}-panel"),
                &s.pixels,
                w,
                h,
                Some(crop),
            );
        }
        // Start search results.
        let mut s = Scene::new(1920, 1080, Controls::DEFAULT);
        let mut st = start::Start::default();
        st.open();
        for ch in "set".chars() {
            st.type_char(ch);
        }
        let g = st.geometry(s.layout);
        let scene = s.pixels.clone();
        st.backdrop.build(&scene, 1920 * 4, g.panel, 8);
        s.bar(&fonts, dark, (0, 0), true, false, false);
        let layout = s.layout;
        let mut c = s.canvas();
        start::draw(
            &mut c,
            &fonts,
            &st,
            layout,
            &start::Frame {
                pointer: (0, 0),
                dark,
                slide: 0,
                transparency: true,
                taskbar_pins: &App::ALL,
            },
        );
        save(
            &format!("start-search-{theme}"),
            &s.pixels,
            1920,
            1080,
            Some(paint_crop(g.panel, 1920, 1080)),
        );
        // Start file search: provider documents and folders under the apps.
        let mut s = Scene::new(1920, 1080, Controls::DEFAULT);
        let mut st = start::Start::default();
        st.open();
        for ch in "note".chars() {
            st.type_char(ch);
        }
        st.file_search = start::FileSearch::Ready { partial: false };
        st.files = vec![
            start::FileResult {
                folder: false,
                name: "notes.txt".into(),
                path: r"C:\Users\1001\Documents".into(),
            },
            start::FileResult {
                folder: false,
                name: "Meeting notes.md".into(),
                path: r"C:\Users\1001\Documents\Work".into(),
            },
            start::FileResult {
                folder: true,
                name: "Notes archive".into(),
                path: r"C:\Users\1001".into(),
            },
        ];
        let g = st.geometry(s.layout);
        let scene = s.pixels.clone();
        st.backdrop.build(&scene, 1920 * 4, g.panel, 8);
        s.bar(&fonts, dark, (0, 0), true, false, false);
        let layout = s.layout;
        let mut c = s.canvas();
        start::draw(
            &mut c,
            &fonts,
            &st,
            layout,
            &start::Frame {
                pointer: (0, 0),
                dark,
                slide: 0,
                transparency: true,
                taskbar_pins: &App::ALL,
            },
        );
        save(
            &format!("start-file-search-{theme}"),
            &s.pixels,
            1920,
            1080,
            Some(paint_crop(g.panel, 1920, 1080)),
        );
        // Snap Assist: the right half offers the other open windows.
        {
            use door_graphics_core::{Engine, Limits, Owner, Role, SurfaceDescriptor, Theme};
            let mut s = Scene::new(1920, 1080, Controls::DEFAULT);
            let area = s.layout.work_area();
            let mut engine = Engine::new(1920, 1080, Theme::Light, Limits::default()).unwrap();
            engine.set_work_area(area).unwrap();
            let mut candidates = Vec::new();
            for (i, (title, app, color)) in [
                ("Notes - desktop.txt", App::Notes, [255u8, 255, 255, 255]),
                ("Settings", App::Settings, [243, 243, 243, 255]),
                ("Terminal", App::Terminal, [12, 12, 12, 255]),
            ]
            .into_iter()
            .enumerate()
            {
                let owner = Owner {
                    pid: i as u64 + 1,
                    session: 1,
                    epoch: 1,
                };
                let id = engine
                    .create_window(
                        owner,
                        Role::TopLevel,
                        Rect::new(100, 100, 640, 432).unwrap(),
                    )
                    .unwrap();
                engine.acknowledge_configure(owner, id, 1).unwrap();
                let buffer = engine
                    .register_buffer(
                        owner,
                        SurfaceDescriptor::new(640, 400, 2560, 640 * 400 * 4).unwrap(),
                    )
                    .unwrap();
                engine.attach(owner, id, buffer, 1).unwrap();
                let mut frame = color.repeat(640 * 400);
                for row in frame.chunks_exact_mut(2560).take(48) {
                    row.copy_from_slice(&[200u8, 120, 0, 255].repeat(640));
                }
                engine.commit(owner, id, frame).unwrap();
                candidates.push(door_window::snap_assist::Candidate::new(
                    id,
                    owner,
                    title,
                    Some(app),
                ));
            }
            let mut assist =
                door_window::snap_assist::Assist::new(0, 0, area, candidates, 0, true).unwrap();
            assert!(assist.refresh(&engine));
            assist.selected = Some(0);
            let left = snap::zone_rect(area, 0, 0).unwrap();
            let mut c = s.canvas();
            let _ = c.fill_rect(
                left,
                if dark {
                    [32, 32, 32, 255]
                } else {
                    [249, 249, 249, 255]
                },
            );
            assist.draw(&mut c, &fonts, dark, (0, 0), 0);
            s.bar(&fonts, dark, (0, 0), false, false, false);
            save(&format!("snap-assist-{theme}"), &s.pixels, 1920, 1080, None);
        }
        // Start > Recent with the provider's recent items.
        let mut s = Scene::new(1920, 1080, Controls::DEFAULT);
        let mut st = start::Start::default();
        st.open();
        st.recent = [
            (true, "Documents", r"C:\Users\1001"),
            (false, "desktop.txt", r"C:\Users\1001\notes"),
            (false, "sunrise.png", r"C:\Users\1001\Pictures"),
            (
                false,
                "Quarterly report.md",
                r"C:\Users\1001\Documents\Work",
            ),
            (false, "budget.csv", r"C:\Users\1001\Documents"),
            (false, "archive.zip", r"C:\Users\1001\Downloads"),
        ]
        .into_iter()
        .map(|(folder, name, path)| start::FileResult {
            folder,
            name: name.into(),
            path: path.into(),
        })
        .collect();
        let g = st.geometry(s.layout);
        let scene = s.pixels.clone();
        st.backdrop.build(&scene, 1920 * 4, g.panel, 8);
        s.bar(&fonts, dark, (0, 0), true, false, false);
        let layout = s.layout;
        let mut c = s.canvas();
        start::draw(
            &mut c,
            &fonts,
            &st,
            layout,
            &start::Frame {
                pointer: (0, 0),
                dark,
                slide: 0,
                transparency: true,
                taskbar_pins: &App::ALL,
            },
        );
        save(
            &format!("start-recent-{theme}"),
            &s.pixels,
            1920,
            1080,
            Some(paint_crop(g.panel, 1920, 1080)),
        );
        // Quick settings and calendar.
        for flyout in [tray::Flyout::Devices, tray::Flyout::Calendar] {
            let mut s = Scene::new(1920, 1080, Controls::DEFAULT);
            let mut state = tray::State::default();
            state.update(time());
            state.open = Some(flyout);
            let bounds = state.bounds(1920, 1080).unwrap();
            let scene = s.pixels.clone();
            state
                .backdrop
                .build(&scene, 1920 * 4, paint_crop(bounds, 1920, 1080), 8);
            s.bar(
                &fonts,
                dark,
                (0, 0),
                false,
                flyout == tray::Flyout::Devices,
                flyout == tray::Flyout::Calendar,
            );
            let layout = s.layout;
            let mut c = s.canvas();
            tray::draw_flyout(
                &mut c,
                &fonts,
                &state,
                flyout,
                layout,
                &tray::Frame {
                    dark,
                    pointer: (0, 0),
                    slide: 0,
                    transparency: true,
                },
            );
            let name = if flyout == tray::Flyout::Devices {
                "quick"
            } else {
                "calendar"
            };
            save(
                &format!("{name}-{theme}"),
                &s.pixels,
                1920,
                1080,
                Some(Rect::new(1920 - 480, 1080 - 640, 480, 640).unwrap()),
            );
        }
        // Desktop context menu with the View submenu open, and a jump list.
        let mut s = Scene::new(1280, 720, Controls::DEFAULT);
        s.bar(&fonts, dark, (0, 0), false, false, false);
        let area = s.layout.output();
        let mut m = menus::desktop(false, true);
        m.place(&fonts, Anchor::Point(500, 200), area);
        let mut sub = menus::desktop_view(menu::Style::Fluent, true);
        let row = m.row(0).unwrap();
        sub.place(&fonts, Anchor::Beside(row), area);
        m.focus = Some(0);
        m.child = Some(Box::new(sub));
        let mut jump = menus::jump_list(App::Notes, true, 1, false);
        let notes = s.layout.slot(taskbar::Slot::App(1), 6);
        jump.place(&fonts, Anchor::Above(notes), area);
        let palette = door_window::paint::Palette::new(dark);
        let mut c = s.canvas();
        menu::draw(&mut c, &fonts, &m, &palette, (row.x() + 30, row.y() + 40));
        menu::draw(&mut c, &fonts, &jump, &palette, (0, 0));
        save(&format!("menus-{theme}"), &s.pixels, 1280, 720, None);
        // Start menus: power, account and the All view selector; Win+X.
        let mut s = Scene::new(1920, 1080, Controls::DEFAULT);
        let mut st = start::Start::default();
        st.open();
        st.view = start::View::List;
        let g = st.geometry(s.layout);
        st.scroll = g.max_scroll() / 2;
        let scene = s.pixels.clone();
        st.backdrop.build(&scene, 1920 * 4, g.panel, 8);
        s.bar(&fonts, dark, (0, 0), true, false, false);
        let layout = s.layout;
        let area = layout.output();
        let power_rect = st.rect(&g, &fonts, start::Target::Power).unwrap();
        let user_rect = st.rect(&g, &fonts, start::Target::User).unwrap();
        let mut power = menus::power();
        power.place(&fonts, Anchor::AboveRight(power_rect), area);
        let mut user = menus::user();
        user.place(&fonts, Anchor::AboveLeft(user_rect), area);
        let mut quick = menus::quick_link();
        quick.place(
            &fonts,
            Anchor::AboveLeft(layout.slot(taskbar::Slot::Start, 6)),
            area,
        );
        let mut c = s.canvas();
        start::draw(
            &mut c,
            &fonts,
            &st,
            layout,
            &start::Frame {
                pointer: (0, 0),
                dark,
                slide: 0,
                transparency: true,
                taskbar_pins: &App::ALL,
            },
        );
        let palette = door_window::paint::Palette::new(dark);
        menu::draw(&mut c, &fonts, &power, &palette, (0, 0));
        menu::draw(&mut c, &fonts, &user, &palette, (0, 0));
        menu::draw(&mut c, &fonts, &quick, &palette, (0, 0));
        save(&format!("start-menus-{theme}"), &s.pixels, 1920, 1080, None);
        // No search results.
        let mut s = Scene::new(1280, 720, Controls::DEFAULT);
        let mut st = start::Start::default();
        st.open();
        for ch in "qqq".chars() {
            st.type_char(ch);
        }
        let layout = s.layout;
        let mut c = s.canvas();
        start::draw(
            &mut c,
            &fonts,
            &st,
            layout,
            &start::Frame {
                pointer: (0, 0),
                dark,
                slide: 0,
                transparency: false,
                taskbar_pins: &App::ALL,
            },
        );
        save(
            &format!("start-empty-search-{theme}"),
            &s.pixels,
            1280,
            720,
            None,
        );
        // 26H2: pins-only Start without the account, and the Run dialog.
        let pins_only = Controls::DEFAULT
            .with(Controls::START_HIDE_RECENT, true)
            .with(Controls::START_HIDE_ALL, true)
            .with(Controls::START_HIDE_ACCOUNT, true);
        let mut s = Scene::new(1280, 720, pins_only);
        let mut st = start::Start::default();
        st.open();
        let layout = s.layout;
        let mut c = s.canvas();
        start::draw(
            &mut c,
            &fonts,
            &st,
            layout,
            &start::Frame {
                pointer: (0, 0),
                dark,
                slide: 0,
                transparency: false,
                taskbar_pins: &App::ALL,
            },
        );
        save(
            &format!("start-pins-only-{theme}"),
            &s.pixels,
            1280,
            720,
            None,
        );
        let mut s = Scene::new(1280, 720, Controls::DEFAULT);
        s.bar(&fonts, dark, (0, 0), false, false, false);
        let mut run = door_window::run::Run {
            history: vec!["cmd".into(), "ms-settings:display".into(), "notepad".into()],
            ..Default::default()
        };
        run.show();
        for ch in "no".chars() {
            run.type_char(ch);
        }
        let layout = s.layout;
        let mut c = s.canvas();
        door_window::run::draw(&mut c, &fonts, &run, layout, dark, (0, 0));
        save(
            &format!("run-suggestions-{theme}"),
            &s.pixels,
            1280,
            720,
            None,
        );
        let mut s = Scene::new(1280, 720, Controls::DEFAULT);
        run.text.clear();
        run.expanded = true;
        let mut c = s.canvas();
        door_window::run::draw(&mut c, &fonts, &run, layout, dark, (0, 0));
        save(&format!("run-recent-{theme}"), &s.pixels, 1280, 720, None);
        // Snap layouts flyout.
        let mut s = Scene::new(1280, 720, Controls::DEFAULT);
        let f = snap::drag_flyout(s.layout.work_area());
        let mut c = s.canvas();
        snap::draw(&mut c, f, Some((1, 0)), dark, Some(App::Settings));
        save(
            &format!("snap-{theme}"),
            &s.pixels,
            1280,
            720,
            Some(Rect::new(f.x() - 20, 0, f.width() + 40, f.height() + 40).unwrap()),
        );
        // Win+V clipboard history: off, empty, and with items and See more.
        clipboard_history(&fonts, dark, theme);
    }
}
/// Trusted environment stand-in for the clipboard broker (one app, focused).
struct ReviewEnv;
impl door_window::clipboard::Env for ReviewEnv {
    fn locked(&self) -> bool {
        false
    }
    fn focused(&self, _: door_window::graphics::Owner) -> bool {
        true
    }
    fn owns(&self, _: door_window::graphics::Owner, _: u64) -> bool {
        true
    }
    fn settings_client(&self, _: door_window::graphics::Owner) -> bool {
        true
    }
    fn snapshot(
        &mut self,
        _: u64,
        _: u64,
        _: &mut [u8],
    ) -> Result<(), door_service_protocol::Code> {
        Err(door_service_protocol::Code::Unsupported)
    }
    fn now(&self) -> u64 {
        0
    }
}
fn clipboard_history(fonts: &FontAtlas, dark: bool, theme: &str) {
    use door_service_protocol::clipboard as cb;
    use door_window::clipboard_history::Panel;
    use door_window::clipboard_view::{draw, geometry};
    let owner = door_window::graphics::Owner {
        pid: 9,
        session: 1,
        epoch: 1,
    };
    let mut broker = door_window::clipboard::Broker::new();
    broker.listen(owner);
    let mut env = ReviewEnv;
    let mut copy = |broker: &mut door_window::clipboard::Broker, text: &str| {
        let word = |v: u32| v.to_le_bytes();
        broker
            .request(&mut env, owner, cb::BEGIN, &1u64.to_le_bytes(), None)
            .ok()
            .unwrap();
        let mut f = word(cb::TEXT).to_vec();
        f.extend(word(text.len() as u32));
        broker
            .request(&mut env, owner, cb::FORMAT, &f, None)
            .ok()
            .unwrap();
        let mut w = word(cb::TEXT).to_vec();
        w.extend(word(0));
        w.extend(text.as_bytes());
        broker
            .request(&mut env, owner, cb::WRITE, &w, None)
            .ok()
            .unwrap();
        broker
            .request(&mut env, owner, cb::COMMIT, &[], None)
            .ok()
            .unwrap();
    };
    let mut panel = Panel::default();
    panel.show();
    for state in ["off", "empty", "items"] {
        if state == "empty" {
            broker.apply_settings(cb::SETTING_HISTORY).unwrap();
        }
        if state == "items" {
            for text in [
                "Meeting notes: ship the clipboard broker on Friday",
                r"C:\Users\1001\Documents\Report.txt",
                "fn main() {\n    println!(\"Hello, Doors\");\n}",
                "Thanks!",
            ] {
                copy(&mut broker, text);
            }
            let pinned = broker.history()[2].id;
            broker.toggle_pin(pinned);
            panel.menu = Some(1);
        }
        let mut s = Scene::new(1280, 720, Controls::DEFAULT);
        s.bar(fonts, dark, (0, 0), false, false, false);
        let layout = s.layout;
        let mut c = s.canvas();
        draw(&mut c, fonts, &panel, &broker, layout, dark, (0, 0));
        let crop = paint_crop(geometry(layout, &panel).panel, 1280, 720);
        save(
            &format!("clipboard-history-{state}-{theme}"),
            &s.pixels,
            1280,
            720,
            Some(crop),
        );
    }
    // Drag feedback label ("Copy to Documents") beside the pointer.
    let mut s = Scene::new(1280, 720, Controls::DEFAULT);
    let layout = s.layout;
    let feedback = door_window::dnd::Feedback {
        effect: door_service_protocol::dnd::COPY,
        taskbar: false,
        insert: door_window::dnd::Insert::new("Documents"),
    };
    let badge =
        door_window::clipboard_view::feedback_rect(fonts, &feedback, (600, 300), layout.output())
            .unwrap();
    let mut c = s.canvas();
    door_window::clipboard_view::draw_feedback(&mut c, fonts, &feedback, badge, dark);
    save(
        &format!("drag-copy-{theme}"),
        &s.pixels,
        1280,
        720,
        Some(paint_crop(badge, 1280, 720)),
    );
}
fn paint_crop(r: Rect, w: u32, h: u32) -> Rect {
    door_window::paint::grow(r, 24, Rect::new(0, 0, w, h).unwrap())
}
