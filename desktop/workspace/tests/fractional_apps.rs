//! Settings and Terminal painted (Files and Notes: `raster_alignment.rs`) at 150% exactly as the
//! native runtime paints them (library View, 150% atlases): every 1 px line
//! is snapped (no half-alpha line pairs) and the frames contain crisp lines.
//! The Start search host draws nothing, so it has no frame to check.
#![allow(dead_code, unused_imports)]
extern crate alloc;
use alloc::{boxed::Box, format, string::String};
use door_abi::platform::{StartupV2, inherited, mode};
use door_graphics_core::Scale;
use door_graphics_text::FontAtlas;
use door_service_client::{Error, Storage, window::WindowClient};
use door_service_protocol::{Code, window as w};
use door_workspace::raster_check::{blurred_lines, crisp_lines};
use door_workspace::runtime::WindowOptions;
use door_workspace::{icons, menu, ui};
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/terminal.rs"]
mod terminal;

const PERCENT: u32 = 150;
fn zoom() -> Scale {
    Scale::from_percent(PERCENT).unwrap()
}
fn ui_fonts() -> FontAtlas {
    let atlas = door_workspace::assets::get("fonts/segoe-ui/hinted-150.dfa");
    FontAtlas::from_packed_percent(atlas, PERCENT).unwrap()
}
fn mono_fonts() -> FontAtlas {
    FontAtlas::monospace_reference_percent(PERCENT).unwrap()
}
/// Paint `size` logical pixels at 150% and check the frame.
fn check(name: &str, size: (u32, u32), dark: bool, draw: impl FnOnce(&mut ui::View<'_>)) {
    check_frame(name, size, dark, true, draw)
}
/// `crisp`: the frame must also show at least one crisp 1 px line (false
/// for views that, as on Windows, have none).
fn check_frame(
    name: &str,
    size: (u32, u32),
    dark: bool,
    crisp: bool,
    draw: impl FnOnce(&mut ui::View<'_>),
) {
    let (fonts, mono) = (ui_fonts(), mono_fonts());
    let images = ui::ImageCache::default();
    let (w, h) = (zoom().len(size.0), zoom().len(size.1));
    let mut pixels = alloc::vec![0; (w * h * 4) as usize];
    let mut v = ui::View::new_zoomed(&mut pixels, w, h, &fonts, zoom())
        .with_images(&images)
        .with_monospace(Some(&mono));
    assert_eq!((v.width, v.height), size, "{name}: logical size");
    v.palette.dark = dark;
    draw(&mut v);
    assert!(
        pixels
            .chunks_exact(4)
            .all(|p| p[..3].iter().all(|c| *c <= p[3])),
        "{name}: premultiplied"
    );
    let (w, h) = (w as usize, h as usize);
    assert_eq!(
        blurred_lines(&pixels, w, h),
        0,
        "{name}: half-alpha line pair"
    );
    assert!(
        !crisp || crisp_lines(&pixels, w, h) > 0,
        "{name}: no crisp lines found"
    );
}
fn startup() -> StartupV2 {
    StartupV2 {
        base: door_abi::Startup {
            pid: 1,
            peer_pid: 0,
            role: 1,
            entry: 0,
            tls: 0,
            stack_top: 0,
            inherited: [door_abi::Inherited::default(); 8],
        },
        identity: door_abi::platform::Identity {
            uid: 1001,
            ..door_abi::platform::Identity::default()
        },
        executable_sha256: [0; 32],
        cwd: 0,
        mode: 0,
        arguments: [0; 128],
        argument_bytes: 0,
        environment: [0; 256],
        environment_bytes: 0,
        standard: [door_abi::Inherited::default(); 3],
    }
}
#[test]
fn settings_paints_snapped_lines_at_150_percent() {
    use door_workspace::settings_layout::Page;
    for page in [
        Page::Home,
        Page::DevicesDrivers,
        Page::Personalization,
        Page::Background,
        Page::Storage,
    ] {
        for dark in [false, true] {
            let mut s = settings::Settings::review(&startup(), page, dark);
            s.configure(1129, 787);
            // Personalization (26H2) shows its preview, the theme grid and
            // cards: no 1 px line on screen, as on Windows.
            let crisp = page != Page::Personalization;
            check_frame(
                &format!("settings {page:?}"),
                (1129, 787),
                dark,
                crisp,
                |v| s.draw(v),
            );
        }
    }
}
#[test]
fn terminal_paints_snapped_lines_at_150_percent() {
    use door_workspace::terminal_model::{Page, Profile};
    let mut t = terminal::Terminal::for_home("/users/1001".into());
    t.configure(1128, 632);
    t.state.new_tab(Profile::PowerShell);
    for page in [Page::Console, Page::Settings, Page::Profiles] {
        t.state.page = page;
        for dark in [false, true] {
            check(&format!("terminal {page:?}"), (1128, 632), dark, |v| {
                t.draw(v)
            });
        }
    }
}
/// Over Mica Settings commits translucent pixels, which the compositor
/// refuses unless every colour channel is within its alpha (premultiplied).
/// The rail and row category icons are bitmaps exported per scale, so each
/// scale's set is checked (a 300 % icon once rounded a channel above alpha
/// and Settings failed to start at 4K).
#[test]
fn settings_over_mica_commits_premultiplied_pixels_at_every_scale() {
    use door_workspace::settings_layout::Page;
    for (percent, key) in [
        (100, "fonts/segoe-ui/hinted-1x.dfa"),
        (150, "fonts/segoe-ui/hinted-150.dfa"),
        (200, "fonts/segoe-ui/hinted-2x.dfa"),
        (300, "fonts/segoe-ui/hinted-3x.dfa"),
    ] {
        let zoom = Scale::from_percent(percent).unwrap();
        let fonts =
            FontAtlas::from_packed_percent(door_workspace::assets::get(key), percent).unwrap();
        let images = ui::ImageCache::default();
        for page in [Page::Home, Page::System, Page::Apps, Page::Personalization] {
            for dark in [false, true] {
                let mut s = settings::Settings::review(&startup(), page, dark);
                s.configure(1129, 787);
                let (w, h) = (zoom.len(1129), zoom.len(787));
                let mut pixels = alloc::vec![0; (w * h * 4) as usize];
                {
                    let mut v =
                        ui::View::new_zoomed(&mut pixels, w, h, &fonts, zoom).with_images(&images);
                    v.palette.dark = dark;
                    v.materials = true;
                    s.draw(&mut v);
                }
                let bad = pixels
                    .chunks_exact(4)
                    .position(|p| p[..3].iter().any(|c| *c > p[3]));
                assert_eq!(bad, None, "{percent}% {page:?} dark={dark}");
            }
        }
    }
}
