//! Host review of the actual Settings painter. Fixtures never stand in for guest evidence.
#![allow(dead_code, unused_imports)]
extern crate alloc;
use alloc::{format, string::String};
use door_service_client::{Error, Storage};
use door_service_protocol::Code;
use door_workspace::menu;
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/tokens.rs"]
mod tokens;
#[path = "../src/ui.rs"]
mod ui;

use door_abi::{
    Inherited, Startup,
    platform::{Identity, StartupV2},
};
use door_service_protocol::desktop::Controls;
use door_workspace::settings_catalog as catalog;
use door_workspace::settings_layout::Page;

fn startup() -> StartupV2 {
    StartupV2 {
        base: Startup {
            pid: 1,
            peer_pid: 0,
            role: 1,
            entry: 0,
            tls: 0,
            stack_top: 0,
            inherited: [Inherited::default(); 8],
        },
        identity: Identity {
            uid: 1001,
            ..Identity::default()
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
fn directory() -> std::path::PathBuf {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/settings-review");
    std::fs::create_dir_all(&directory).unwrap();
    directory
}
/// Paint `s` at `width` x `height` (1x) and optionally write a PPM review.
fn paint(
    s: &settings::Settings,
    width: u32,
    height: u32,
    dark: bool,
    hover: Option<(i32, i32)>,
    name: Option<&str>,
) -> Vec<u8> {
    let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
    let images = ui::ImageCache::default();
    let mut pixels = vec![0; width as usize * height as usize * 4];
    {
        let mut view =
            ui::View::new_scaled(&mut pixels, width, height, &fonts, 1).with_images(&images);
        view.palette.dark = dark;
        view.hover = hover;
        s.draw(&mut view);
    }
    assert!(pixels.chunks_exact(4).all(|p| p[3] == 255), "opaque frame");
    if let Some(name) = name {
        let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
        ppm.extend(pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]));
        std::fs::write(directory().join(format!("{name}.ppm")), ppm).unwrap();
    }
    pixels
}
fn pixel(pixels: &[u8], width: u32, x: i32, y: i32) -> [u8; 4] {
    let at = (y as usize * width as usize + x as usize) * 4;
    pixels[at..at + 4].try_into().unwrap()
}
const TAB: u32 = 15;
/// A pixel column inside the rail selection pill (26H2: x 17..20).
const PILL_X: i32 = door_workspace::settings_layout::Layout::NAV_X + 1;
const RETURN: u32 = 28;
const SPACE: u32 = 57;
const DOWN: u32 = 0x150;
const UP: u32 = 0x148;
fn ch(c: char) -> u32 {
    (c as u32) << 16
}
fn ctrl(scan: u32) -> u32 {
    scan | (door_abi::graphics::CONTROL << 9)
}
fn alt(scan: u32) -> u32 {
    scan | (door_abi::graphics::ALT << 9)
}

/// The pages that replace legacy administration windows paint with the same
/// Windows card vocabulary as every other page, in both themes and at every
/// width, with no target outside the visible main column.
#[test]
fn unified_administration_pages_paint_as_windows_settings_pages() {
    let startup = startup();
    let directory = directory();
    let pages = [
        Page::Registry,
        Page::DevicesDrivers,
        Page::DiskManagement,
        Page::NetworkAdministration,
        Page::Services,
        Page::Environment,
        Page::SystemConfiguration,
        Page::GroupPolicy,
        Page::EventViewer,
        Page::TaskScheduler,
        Page::UsersGroups,
        Page::PermissionsSharing,
        Page::Performance,
        Page::PowerManagement,
        Page::BackupRestore,
        Page::DeveloperTools,
        Page::Fonts,
        Page::SystemAdvanced,
        Page::Microphone,
    ];
    for dark in [false, true] {
        for page in pages {
            let mut settings = settings::Settings::review(&startup, page, dark);
            settings.configure(1472, 952);
            let pixels = paint(&settings, 1472, 952, dark, None, None);
            // Mica background beside the content, like every Windows page.
            let bg = if dark {
                [36, 32, 31, 255]
            } else {
                [250, 247, 244, 255]
            };
            assert_eq!(pixel(&pixels, 1472, 300, 940), bg, "{page:?}");
            for width in [240, 320, 516, 600, 612, 760, 1129] {
                let mut compact = settings::Settings::review(&startup, page, dark);
                compact.configure(width, 720);
                paint(&compact, width, 720, dark, None, None);
                let edge = compact.target(width as i32 - 1, 300);
                assert!(
                    width < 612 || edge.is_none_or(|id| id == 400),
                    "{page:?} {width} {edge:?}"
                );
                assert_eq!(compact.target(width as i32, 200), None);
                if width < 612 {
                    assert_eq!(compact.target(310, 200), None);
                }
            }
            let mut ppm = b"P6\n1472 952\n255\n".to_vec();
            ppm.extend(pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]));
            std::fs::write(
                directory.join(format!(
                    "page-{}-{}.ppm",
                    page as u32,
                    if dark { "dark" } else { "light" }
                )),
                ppm,
            )
            .unwrap();
        }
    }
}

#[test]
fn page_heading_shows_route_ancestors_and_only_ancestors_are_links() {
    let startup = startup();
    for (page, width, dark, name, links) in [
        (Page::Taskbar, 1129u32, false, "heading-taskbar-light", 1),
        (Page::Taskbar, 1129, true, "heading-taskbar-dark", 1),
        (Page::NetworkAdministration, 700, false, "heading-narrow", 0),
        (Page::Registry, 1129, false, "heading-registry", 2),
        (Page::DiskManagement, 1129, false, "heading-disks", 2),
        (Page::Personalization, 1129, false, "heading-top-level", 0),
        (Page::Colors, 1129, false, "heading-colors", 1),
    ] {
        let mut s = settings::Settings::review(&startup, page, dark);
        s.configure(width, 787);
        paint(&s, width, 787, dark, None, Some(name));
        let found: Vec<u32> = (310..width as i32)
            .step_by(4)
            .filter_map(|x| s.target(x, 80))
            .filter(|id| (260..262).contains(id))
            .fold(Vec::new(), |mut v, id| {
                if !v.contains(&id) {
                    v.push(id);
                }
                v
            });
        assert_eq!(found.len(), links, "{name}: ancestor links {found:?}");
        // Activating the nearest ancestor opens the parent page.
        if let Some(last) = found.last() {
            let x = (310..width as i32)
                .step_by(4)
                .find(|x| s.target(*x, 80) == Some(*last))
                .unwrap();
            s.click_with(x, 80, None).unwrap();
            assert_eq!(s.current_page(), page.parent(), "{name}");
        }
    }
}

/// Every ordinary page, both themes, the reference size and narrow widths.
#[test]
fn every_windows_page_paints_in_both_themes_and_at_narrow_widths() {
    let startup = startup();
    for page in Page::ALL {
        for dark in [false, true] {
            let theme = if dark { "dark" } else { "light" };
            let mut s = settings::Settings::review(&startup, page, dark);
            s.configure(1129, 787);
            let name = format!(
                "windows-{}-{theme}",
                page.nav_title().replace([' ', '&'], "").to_lowercase()
            );
            paint(&s, 1129, 787, dark, None, Some(&name));
            for (width, height) in [(612u32, 700u32), (760, 640), (900, 787), (1600, 900)] {
                let mut n = settings::Settings::review(&startup, page, dark);
                n.configure(width, height);
                let name = format!("{name}-{width}");
                paint(
                    &n,
                    width,
                    height,
                    dark,
                    None,
                    (dark && width == 760).then_some(name.as_str()),
                );
                // No content target outside the visible main column.
                assert!(n.target(width as i32 - 1, 300).is_none_or(|id| id == 400));
            }
        }
    }
}

/// Native densities: every asset frame used by the new pages exists at 2x/3x.
#[test]
fn windows_pages_paint_at_native_densities() {
    let startup = startup();
    for scale in [2u32, 3] {
        let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(scale).unwrap();
        let images = ui::ImageCache::default();
        for page in [
            Page::Home,
            Page::About,
            Page::Taskbar,
            Page::Personalization,
            Page::Background,
            Page::InstalledApps,
            Page::Network,
        ] {
            let mut s = settings::Settings::review(&startup, page, scale == 3);
            s.configure(1129, 787);
            let (w, h) = (1129 * scale, 787 * scale);
            let mut pixels = vec![0; w as usize * h as usize * 4];
            let mut view =
                ui::View::new_scaled(&mut pixels, w, h, &fonts, scale).with_images(&images);
            view.palette.dark = scale == 3;
            s.draw(&mut view);
            assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
            if page == Page::About && scale == 2 {
                let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
                ppm.extend(pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]));
                std::fs::write(directory().join("about-light-2x.ppm"), ppm).unwrap();
            }
        }
    }
}

#[test]
fn title_bar_back_search_account_and_rail_match_the_reference_structure() {
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::Home, true);
    s.configure(1129, 787);
    let pixels = paint(&s, 1129, 787, true, None, Some("structure-home-dark"));
    // Search box centred in the caption control, Back at the left.
    assert_eq!(s.target(564, 24), Some(200));
    assert_eq!(s.target(20, 24), Some(201));
    // Rail: Home at 144, 40 px pitch; Personalization is row 4.
    assert_eq!(s.target(100, 150), Some(100));
    assert_eq!(
        s.target(100, 144 + 4 * 40 + 10),
        Some(100 + Page::Personalization as u32)
    );
    // Selection pill in the accent color at the row's left edge.
    assert_eq!(pixel(&pixels, 1129, PILL_X, 144 + 18), [255, 205, 96, 255]);
    // Navigating enables Back; Back returns to Home.
    s.click_with(100, 144 + 40 + 10, None).unwrap();
    assert_eq!(s.current_page(), Page::System);
    s.click_with(20, 24, None).unwrap();
    assert_eq!(s.current_page(), Page::Home);
    s.key_with(alt(0x14d), None).unwrap();
    assert_eq!(s.current_page(), Page::System);
    s.key_with(alt(0x14b), None).unwrap();
    assert_eq!(s.current_page(), Page::Home);
}

#[test]
fn search_suggestions_navigate_by_keyboard_and_pointer() {
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1129, 787);
    s.key_with(ctrl(33), None).unwrap();
    for c in "dark mode".chars() {
        s.key_with(ch(c), None).unwrap();
    }
    paint(&s, 1129, 787, false, None, Some("search-suggestions"));
    s.key_with(RETURN, None).unwrap();
    assert_eq!(s.current_page(), Page::Colors);
    // Pointer: focus search, type, click the first suggestion row.
    s.click_with(564, 24, None).unwrap();
    for c in "taskbar".chars() {
        s.key_with(ch(c), None).unwrap();
    }
    let (x, y, _, pitch) = door_workspace::settings_navigation::suggestion_box(
        door_workspace::settings_layout::Layout {
            width: 1129,
            height: 787,
        },
    )
    .unwrap();
    assert_eq!(s.target(x + 40, y + 4 + pitch / 2), Some(150));
    s.click_with(x + 40, y + 4 + pitch / 2, None).unwrap();
    assert_eq!(s.current_page(), Page::Taskbar);
}

#[test]
fn bound_controls_persist_and_follow_live_controls_changes() {
    let startup = startup();
    // Colors: Tab focuses "Choose your mode"; arrows change it like WinUI.
    let mut s = settings::Settings::review(&startup, Page::Colors, false);
    s.configure(1129, 787);
    s.key_with(TAB, None).unwrap();
    paint(&s, 1129, 787, false, None, Some("colors-focus-light"));
    s.key_with(DOWN, None).unwrap();
    assert_eq!(s.pending_theme, Some(1));
    s.key_with(UP, None).unwrap();
    assert_eq!(s.pending_theme, Some(0));
    // Transparency effects: second item; Space toggles REDUCE_TRANSPARENCY.
    s.key_with(TAB, None).unwrap();
    s.key_with(SPACE, None).unwrap();
    assert_eq!(s.controls_bits(), 48 | Controls::REDUCE_TRANSPARENCY);
    // A CONTROLS_CHANGED from elsewhere updates the page live.
    s.pending_controls = None;
    s.controls_changed(Controls::from_bits(48).unwrap());
    assert_eq!(s.controls_bits(), 48);
    // Combo by pointer: open the list, pick Dark.
    let mut s = settings::Settings::review(&startup, Page::Colors, true);
    s.configure(1129, 787);
    let field = catalog::points::item(Page::Colors, 1129, 787, [0; catalog::List::COUNT], |i| {
        i.action == catalog::Action::Combo(catalog::Choice::Mode)
    })
    .unwrap();
    s.click_with(field.0, field.1, None).unwrap();
    assert!(s.menu_open());
    paint(
        &s,
        1129,
        787,
        true,
        Some(field),
        Some("colors-combo-open-dark"),
    );
    // The open list aligns the current option (Dark) with the field.
    s.click_with(field.0, field.1 - 36, None).unwrap();
    assert_eq!(s.pending_theme, Some(0));
    // Context menu (26H2 order): File actions, Show more options, Properties
    // placement, then inline actions and the classic menu.
    let mut s = settings::Settings::review(&startup, Page::ContextMenu, false);
    s.configure(1129, 787);
    s.key_with(TAB, None).unwrap();
    s.key_with(SPACE, None).unwrap();
    assert_eq!(s.controls_bits(), 48 | Controls::CONTEXT_HIDE_FILE_ACTIONS);
    s.key_with(TAB, None).unwrap();
    s.key_with(SPACE, None).unwrap();
    let hidden = 48 | Controls::CONTEXT_HIDE_FILE_ACTIONS | Controls::CONTEXT_HIDE_MORE_OPTIONS;
    assert_eq!(s.controls_bits(), hidden);
    s.key_with(TAB, None).unwrap();
    s.key_with(UP, None).unwrap();
    assert_eq!(s.controls_bits(), hidden & !32, "Properties at the top");
    s.key_with(TAB, None).unwrap();
    s.key_with(SPACE, None).unwrap();
    assert_eq!(s.controls_bits(), hidden & !32 & !16, "inline actions off");
    s.key_with(TAB, None).unwrap();
    s.key_with(SPACE, None).unwrap();
    assert_eq!(
        s.controls_bits(),
        (hidden & !32 & !16) | Controls::CLASSIC_CONTEXT_MENU
    );
    // Visual effects: Animation effects Off sets REDUCE_MOTION.
    let mut s = settings::Settings::review(&startup, Page::VisualEffects, false);
    s.configure(1129, 787);
    s.key_with(TAB, None).unwrap();
    s.key_with(TAB, None).unwrap();
    s.key_with(SPACE, None).unwrap();
    assert_eq!(s.controls_bits(), 48 | Controls::REDUCE_MOTION);
    // Themes: desktop icons through the expander.
    let mut s = settings::Settings::review(&startup, Page::Themes, false);
    s.configure(1129, 787);
    // Background, Color and Mouse cursor, the Current theme header, its
    // six theme tiles, the Desktop icon settings header, then its toggle.
    for _ in 0..12 {
        s.key_with(TAB, None).unwrap();
    }
    s.key_with(SPACE, None).unwrap();
    assert_eq!(s.controls_bits(), 48 | Controls::SHOW_DESKTOP_ICONS);
    // Start (26H2): size through its list, then the section toggles.
    let mut s = settings::Settings::review(&startup, Page::Start, false);
    s.configure(1129, 787);
    paint(&s, 1129, 787, false, None, Some("start-26h2-light"));
    s.key_with(TAB, None).unwrap();
    s.key_with(RETURN, None).unwrap();
    s.key_with(DOWN, None).unwrap();
    s.key_with(RETURN, None).unwrap();
    assert_eq!(
        Controls::from_bits(s.controls_bits()).unwrap().start_size(),
        1
    );
    for flag in [
        Controls::START_HIDE_PINNED,
        Controls::START_HIDE_RECENT,
        Controls::START_HIDE_ALL,
        Controls::START_HIDE_ACCOUNT,
        // Recommended: recently added apps always show (locked on).
        Controls::START_HIDE_RECENT_FILES,
    ] {
        let before = s.controls_bits();
        s.key_with(TAB, None).unwrap();
        s.key_with(SPACE, None).unwrap();
        assert_eq!(s.controls_bits(), before | flag);
    }
    // Top and side taskbars force the search icon: the box is not offered.
    let mut s = settings::Settings::review(&startup, Page::Taskbar, false);
    s.configure(1129, 787);
    s.review_state(0, 48 | 1);
    s.key_with(TAB, None).unwrap();
    s.key_with(TAB, None).unwrap();
    s.key_with(DOWN, None).unwrap();
    assert_eq!(
        s.controls_bits(),
        48 | 1,
        "Search box unavailable at the top"
    );
    paint(
        &s,
        1129,
        787,
        true,
        None,
        Some("taskbar-top-search-icon-dark"),
    );
    let mut s = settings::Settings::review(&startup, Page::Taskbar, false);
    s.configure(1129, 787);
    paint(&s, 1129, 787, false, None, Some("taskbar-expanders-light"));
    // First focusable: the Taskbar items expander, then its Search combo.
    s.key_with(TAB, None).unwrap();
    s.key_with(TAB, None).unwrap();
    s.key_with(UP, None).unwrap();
    assert!(
        Controls::from_bits(s.controls_bits())
            .unwrap()
            .search_icon_only()
    );
}

#[test]
fn wheel_scrolls_page_and_rail_smoothly_and_reduce_motion_jumps() {
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::System, false);
    s.configure(1129, 600);
    assert!(s.wheel(700, 400, -2));
    let (value, target) = s.scroll_offset();
    assert_eq!((value, target), (0, 96));
    let mut now = 1;
    let mut frames = 0;
    while s.tick(now) {
        now += 1;
        frames += 1;
        assert!(frames < 40);
    }
    assert_eq!(s.scroll_offset(), (96, 96));
    assert!(frames >= 3, "animated over several frames");
    assert!(s.wheel(700, 400, 5));
    loop {
        now += 1;
        if !s.tick(now) {
            break;
        }
    }
    assert_eq!(s.scroll_offset().0, 0);
    assert!(!s.wheel(700, 400, 1), "already at the top");
    paint(&s, 1129, 600, false, None, None);
    // Reduce motion: the offset jumps without animation.
    s.review_state(0, 48 | Controls::REDUCE_MOTION);
    assert!(s.wheel(700, 400, -1));
    assert_eq!(s.scroll_offset(), (48, 48));
    // The rail scrolls on its own when the pointer is over it.
    let mut r = settings::Settings::review(&startup, Page::Registry, false);
    r.configure(1129, 600);
    assert!(r.wheel(100, 400, -3));
    assert_eq!(r.scroll_offset(), (0, 0), "page offset unchanged");
}

#[test]
fn installed_apps_list_real_apps_and_open_launches_through_the_menu() {
    let startup = startup();
    for (width, height) in [(1129u32, 787u32), (1920, 1032)] {
        let mut s = settings::Settings::review(&startup, Page::InstalledApps, false);
        s.configure(width, height);
        paint(
            &s,
            width,
            height,
            false,
            None,
            Some(&format!("installed-apps-{width}")),
        );
        let content = door_workspace::settings_layout::Layout { width, height }.content_width();
        let hero = door_workspace::settings_catalog::Rect::new(310, catalog::TOP, content, 0);
        let row = door_workspace::settings_catalog::app_row(hero, 3);
        let button = door_workspace::settings_catalog::app_menu_button(row);
        let (bx, by) = (button.x + 16, button.center_y());
        assert!(s.target(bx, by).is_some_and(|id| id >= 3000));
        s.click_with(bx, by, None).unwrap();
        assert!(s.menu_open());
        // Uninstall/Advanced options have no provider: disabled.
        let (ox, oy) = (bx - 100, by + 16 + 4 + 4 + 18);
        // The points the guest settings-reference scenario derives.
        assert_eq!(
            door_workspace::settings_catalog::points::installed_app_open(3, width, height),
            ((bx, by), (ox, oy))
        );
        if width == 1920 {
            assert_eq!((bx, by), (1270, 416));
        }
        s.click_with(ox, oy + 72, None).unwrap();
        assert!(s.menu_open(), "disabled Uninstall keeps the menu open");
        assert_eq!(s.pending_launch, None);
        s.click_with(ox, oy, None).unwrap();
        // Apps are listed by name, as Windows sorts them: row 3 is Settings.
        assert_eq!(s.pending_launch, Some(3));
        assert!(!s.menu_open());
        s.pending_launch = None;
        // Terminal sorts last (row 5).
        let ((tx, ty), (tox, toy)) =
            door_workspace::settings_catalog::points::installed_app_open(5, width, height);
        s.click_with(tx, ty, None).unwrap();
        assert!(s.menu_open());
        s.click_with(tox, toy, None).unwrap();
        assert_eq!(s.pending_launch, Some(9));
    }
}

#[test]
fn guest_scenario_keyboard_paths_stay_valid() {
    let startup = startup();
    // settings-reference: Ctrl+F, query + Return, Esc lands on each page.
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1920, 1032);
    for (query, page) in [
        ("drivers", Page::DevicesDrivers),
        ("disk management", Page::DiskManagement),
        ("services", Page::Services),
        ("registry", Page::Registry),
        ("group policy", Page::GroupPolicy),
        ("task scheduler", Page::TaskScheduler),
        ("installed apps", Page::InstalledApps),
        ("colors", Page::Colors),
    ] {
        s.key_with(ctrl(33), None).unwrap();
        for c in query.chars() {
            s.key_with(ch(c), None).unwrap();
        }
        s.key_with(RETURN, None).unwrap();
        s.key_with(1, None).unwrap();
        assert_eq!(s.current_page(), page, "{query}");
    }
    // Theme switch used by the scenario: Colors, Tab, Down / Up.
    s.key_with(TAB, None).unwrap();
    s.key_with(DOWN, None).unwrap();
    assert_eq!(s.pending_theme, Some(1));
    s.key_with(ctrl(33), None).unwrap();
    for c in "colors".chars() {
        s.key_with(ch(c), None).unwrap();
    }
    s.key_with(RETURN, None).unwrap();
    s.key_with(1, None).unwrap();
    s.key_with(TAB, None).unwrap();
    s.key_with(UP, None).unwrap();
    assert_eq!(s.pending_theme, Some(0));
}

// ---------------------------------------------------------------------------
// Points and key sequences used by the guest scenarios that drive Settings
// (tools/xtask/src/desktop/{controls,file_mutations,lifecycle}.rs). The wide
// profile is 1280x720 at 1x with a 48 px bottom taskbar, so a maximized
// Settings window is 1280x672 at the screen origin and window-local points are
// screen points; a right-snapped window is 640x672 at x = 640.
// ---------------------------------------------------------------------------
const ESC: u32 = 1;
const PGDN: u32 = 0x151;
const END: u32 = 0x14f;
fn shift(scan: u32) -> u32 {
    scan | (door_abi::graphics::SHIFT << 9)
}
/// `page()` in controls.rs / administration.rs: Esc, Ctrl+F, query, Return, Esc.
fn search_page(s: &mut settings::Settings, query: &str) {
    s.key_with(ESC, None).unwrap();
    s.key_with(ctrl(33), None).unwrap();
    for c in query.chars() {
        s.key_with(ch(c), None).unwrap();
    }
    s.key_with(RETURN, None).unwrap();
    s.key_with(ESC, None).unwrap();
}
fn keys(s: &mut settings::Settings, codes: &[u32]) {
    for code in codes {
        s.key_with(*code, None).unwrap();
    }
}
fn settle(s: &mut settings::Settings, now: &mut u64) {
    for _ in 0..200 {
        *now += 1;
        if !s.tick(*now) {
            return;
        }
    }
    panic!("scroll animation did not settle");
}
fn bits(s: &settings::Settings) -> u32 {
    s.controls_bits()
}

#[test]
fn controls_scenario_taskbar_and_context_menu_key_sequences() {
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1280, 672);
    search_page(&mut s, "taskbar");
    assert_eq!(s.current_page(), Page::Taskbar);
    // Taskbar behaviors is collapsed on entry: Tab x5 reaches its header
    // (Taskbar items is open: Search, Task view), Space opens it, Tab x2
    // reaches Taskbar position.
    keys(&mut s, &[TAB, TAB, TAB, TAB, TAB, SPACE, TAB, TAB]);
    // Edge 0: open the list and re-choose Bottom for a fresh marker.
    keys(&mut s, &[RETURN]);
    assert!(s.menu_open(), "{:?}", s.focused_action());
    keys(&mut s, &[RETURN]);
    assert_eq!(bits(&s), 48);
    for edge in 1..4u32 {
        keys(&mut s, &[DOWN]);
        assert_eq!(bits(&s), 48 + edge);
        // The maximized window follows the work area; focus is kept.
        let (w, h) = if edge >= 2 { (1232, 720) } else { (1280, 672) };
        s.configure(w, h);
    }
    // Alignment (Shift+Tab, Up = Left/Top) then Taskbar size (Tab x2, Up = Small).
    keys(&mut s, &[shift(TAB), UP]);
    assert_eq!(bits(&s), 55);
    keys(&mut s, &[TAB, TAB, UP]);
    assert_eq!(bits(&s), 63);
    // Context menu (26H2 order): File actions, Show more options,
    // Properties placement, inline actions, classic menu.
    s.configure(1232, 720);
    search_page(&mut s, "context menu");
    assert_eq!(s.current_page(), Page::ContextMenu);
    keys(&mut s, &[TAB, SPACE]);
    assert_eq!(bits(&s), 63 | Controls::CONTEXT_HIDE_FILE_ACTIONS);
    keys(&mut s, &[SPACE]);
    assert_eq!(bits(&s), 63);
    keys(&mut s, &[TAB, SPACE]);
    assert_eq!(bits(&s), 63 | Controls::CONTEXT_HIDE_MORE_OPTIONS);
    keys(&mut s, &[SPACE]);
    assert_eq!(bits(&s), 63);
    keys(&mut s, &[TAB, UP]);
    assert_eq!(bits(&s), 31, "Properties at the top");
    keys(&mut s, &[TAB, SPACE]);
    assert_eq!(bits(&s), 15, "inline actions off");
    keys(&mut s, &[SPACE]);
    assert_eq!(bits(&s), 31);
    keys(&mut s, &[shift(TAB), DOWN]);
    assert_eq!(bits(&s), 63, "Properties at the bottom");
    // Reset: Taskbar again (behaviors collapsed again on entry).
    search_page(&mut s, "taskbar");
    keys(&mut s, &[TAB, TAB, TAB, TAB, TAB, SPACE, TAB, TAB]);
    keys(&mut s, &[RETURN, UP, UP, UP, RETURN]);
    assert_eq!(bits(&s), 60, "Bottom through the list");
    s.configure(1280, 672);
    keys(&mut s, &[shift(TAB), DOWN]);
    assert_eq!(bits(&s), 56, "Center");
    keys(&mut s, &[TAB, TAB, DOWN]);
    assert_eq!(bits(&s), 48, "Default size");
    // Maximized Settings background at window (300, 16) for every work area.
    for (w, h) in [(1280u32, 672u32), (1232, 720)] {
        let pixels = paint(&s, w, h, false, Some((1100, 600)), None);
        assert_eq!(pixel(&pixels, w, 300, 16), [250, 247, 244, 255]);
    }
}

#[test]
fn file_mutations_scenario_inline_actions_key_sequence() {
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1129, 787);
    // settings_mode(c, true) first, then the context-menu step.
    search_page(&mut s, "colors");
    keys(&mut s, &[TAB, DOWN]);
    assert_eq!(s.pending_theme, Some(1));
    s.key_with(ctrl(33), None).unwrap();
    for c in "context menu".chars() {
        s.key_with(ch(c), None).unwrap();
    }
    keys(&mut s, &[RETURN, ESC, TAB, TAB, TAB, TAB, SPACE]);
    assert_eq!(bits(&s), 32, "inline actions off");
    // Later, after Explorer renames, Space on the same focused toggle.
    s.dismiss_menu();
    keys(&mut s, &[SPACE]);
    assert_eq!(bits(&s), 48);
}

#[test]
fn lifecycle_scenario_points_follow_the_settings_layout() {
    // tools/xtask/src/desktop/lifecycle.rs derives every Settings point from
    // `settings_catalog::points`; this drives the same points through the
    // real model and painter. Maximized: 1280x672 at the origin; the snaps
    // are 640x672.
    use door_workspace::settings_catalog::{Fact, points};
    let (w, h) = (1280u32, 672u32);
    let (nw, nh) = (640u32, 672u32);
    let startup = startup();
    let mut now = 0;
    let accent = [203, 103, 0, 255];
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(w, h);
    // Pinned: the rail starts below the account block at every size, and the
    // title-bar search is centred in the caption.
    assert_eq!(points::rail_row(0), (156, 162));
    assert_eq!(points::rail_pill(1), (PILL_X, 202));
    assert_eq!(points::search(w, h), Some((640, 24)));
    assert_eq!(points::heading_background(w), (1250, 52));
    // Rail rows: each row selects its category and paints the accent pill.
    for (row, page) in Page::NAV.iter().enumerate() {
        let (x, y) = points::rail_row(row);
        assert_eq!(s.target(x, y), Some(100 + *page as u32));
        s.click_with(x, y, None).unwrap();
        assert_eq!(s.current_page(), *page);
        let pixels = paint(&s, w, h, false, None, None);
        let (px, py) = points::rail_pill(row);
        assert_eq!(pixel(&pixels, w, px, py), accent, "{page:?}");
    }
    let pill = |s: &settings::Settings, (w, h): (u32, u32), row: usize| {
        let pixels = paint(s, w, h, false, None, None);
        let (px, py) = points::rail_pill(row);
        pixel(&pixels, w, px, py)
    };
    search_page(&mut s, "about");
    assert_eq!(s.current_page(), Page::About);
    assert_eq!(pill(&s, (w, h), 1), accent, "System is selected");
    // Apps row, then the Installed apps card.
    let (x, y) = points::rail_row(5);
    s.click_with(x, y, None).unwrap();
    let (x, y) = points::link(Page::Apps, Page::InstalledApps, w, h).unwrap();
    s.click_with(x, y, None).unwrap();
    assert_eq!(s.current_page(), Page::InstalledApps);
    // System row, then the Display card.
    let (x, y) = points::rail_row(1);
    s.click_with(x, y, None).unwrap();
    let (x, y) = points::link(Page::System, Page::Display, w, h).unwrap();
    s.click_with(x, y, None).unwrap();
    assert_eq!(s.current_page(), Page::Display);
    // End, then the left snap: the snap keeps the scroll offset (clamped),
    // the Scale combo wraps below its label (as Windows does) and the
    // resolution and orientation values keep their glyphs, right aligned.
    keys(&mut s, &[END]);
    settle(&mut s, &mut now);
    let wide_scroll = points::end_scroll(Page::Display, w, h);
    assert!(wide_scroll > 0);
    assert_eq!(s.scroll_offset(), (wide_scroll, wide_scroll));
    let wide = paint(&s, w, h, false, None, Some("lifecycle-display-wide"));
    s.configure(nw, nh);
    let narrow_scroll = wide_scroll.min(points::end_scroll(Page::Display, nw, nh));
    assert_eq!(s.scroll_offset(), (narrow_scroll, narrow_scroll));
    let narrow = paint(&s, nw, nh, false, None, Some("lifecycle-display-narrow"));
    let (px, py) = points::rail_pill(1);
    assert_eq!(pixel(&narrow, nw, px, py), accent, "narrow rail");
    // The orientation value: Display's Related settings push the resolution
    // card above the end-scrolled view at the maximized size.
    for fact in [Fact::Orientation] {
        let (wx, wy) = points::value(Page::Display, fact, w, h, wide_scroll).unwrap();
        let (nx, ny) = points::value(Page::Display, fact, nw, nh, narrow_scroll).unwrap();
        // The value's right edge sits CARD_RIGHT inside the card.
        assert_eq!(
            wx,
            door_workspace::settings_layout::Layout::MAIN
                + door_workspace::settings_layout::Layout {
                    width: w,
                    height: h
                }
                .content_width() as i32
                - catalog::CARD_RIGHT
        );
        assert!((0..h as i32).contains(&wy) && (0..nh as i32).contains(&ny));
        let mut ink = 0;
        for dy in -7..5 {
            for dx in 1..=28 {
                let a = pixel(&wide, w, wx - dx, wy + dy);
                let b = pixel(&narrow, nw, nx - dx, ny + dy);
                assert!(a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= 2), "{fact:?}");
                ink += usize::from(a[0] < 200);
            }
        }
        assert!(ink >= 5, "{fact:?} value glyphs");
    }
    s.configure(w, h);
    assert_eq!(pill(&s, (w, h), 1), accent);
    // Ctrl+F "installed" + Return lands on Installed apps (Apps selected).
    s.key_with(ctrl(33), None).unwrap();
    for c in "installed".chars() {
        s.key_with(ch(c), None).unwrap();
    }
    keys(&mut s, &[RETURN]);
    assert_eq!(s.current_page(), Page::InstalledApps);
    assert_eq!(pill(&s, (w, h), 5), accent);
    keys(&mut s, &[ESC]);
    // Title-bar search box by pointer.
    let (sx, sy) = points::search(w, h).unwrap();
    assert_eq!(s.target(sx, sy), Some(200));
    s.click_with(sx, sy, None).unwrap();
    for c in "network".chars() {
        s.key_with(ch(c), None).unwrap();
    }
    keys(&mut s, &[RETURN, ESC]);
    assert_eq!(s.current_page(), Page::Network);
    assert_eq!(pill(&s, (w, h), 3), accent);
    // Home, PgDn: the header stays (search box fill) and Home stays selected.
    let (x, y) = points::rail_row(0);
    s.click_with(x, y, None).unwrap();
    keys(&mut s, &[PGDN]);
    settle(&mut s, &mut now);
    assert!(s.scroll_offset().0 > 0);
    let pixels = paint(&s, w, h, false, Some((x, y)), None);
    assert_eq!(pixel(&pixels, w, sx, sy), [253, 253, 253, 255]);
    assert_eq!(pixel(&pixels, w, PILL_X, y), accent);
    // Installed apps: File Explorer and Notes "..." then Open.
    search_page(&mut s, "installed apps");
    for (index, command) in [(0, 1), (1, 2)] {
        let (more, open) = points::installed_app_open(index, w, h);
        s.pending_launch = None;
        s.click_with(more.0, more.1, None).unwrap();
        assert!(s.menu_open());
        // The lifecycle scenario's probe: the open menu's solid fill.
        let (px, py) = points::installed_app_menu_padding(index, w, h);
        let pixels = paint(&s, w, h, false, None, None);
        assert_eq!(pixel(&pixels, w, px, py), [252, 252, 252, 255]);
        s.click_with(open.0, open.1, None).unwrap();
        assert_eq!(s.pending_launch, Some(command));
        // A close behind this click waits until the launch is sent.
        assert!(s.has_pending_request());
    }
    // Themes with Settings snapped right (640x672 at x = 640).
    s.configure(nw, nh);
    search_page(&mut s, "personalization");
    for (i, def) in door_workspace::settings_layout::THEMES.iter().enumerate() {
        let (x, y) = points::theme_tile(i, nw, nh).unwrap();
        s.pending_theme = None;
        s.click_with(x, y, None).unwrap();
        assert_eq!(s.pending_theme, Some(def.appearance()), "tile {i}");
    }
    // Maximized heading-row background probe in both modes.
    s.configure(w, h);
    search_page(&mut s, "colors");
    let (bx, by) = points::heading_background(w);
    for (dark, expected) in [(false, [250, 247, 244, 255]), (true, [36, 32, 31, 255])] {
        let pixels = paint(&s, w, h, dark, None, None);
        assert_eq!(pixel(&pixels, w, bx, by), expected);
    }
}

/// Window-management probes in lifecycle.rs (`wide-max`) and the
/// desktop-motion exposed point (`window_points::motion`).
#[test]
fn window_management_probes_hit_the_settings_background() {
    let startup = startup();
    let bg = [250, 247, 244, 255];
    // Maximized (work area 1280x672) Home page: (1270, 630) right of the
    // cards and left of the overlay scrollbar.
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1280, 672);
    let pixels = paint(&s, 1280, 672, false, Some((1211, 20)), None);
    assert_eq!(pixel(&pixels, 1280, 1270, 630), bg);
    // Initial placement (`graphics::initial_window`, 70% x 78% of the work
    // area): 896x524 at 1280x720 (also 2560x1440 at 2x and 3840x2160 at 3x),
    // 717x562 at 1024x768, 896x587 at 1280x800 (2560x1600 at 2x) and the
    // requested 1129x787 at 1920x1080.
    // Window (75, 130), between the account block and the rail, is the
    // background at every size: search lives only in the title bar
    // (`window_points::SETTINGS_BACKGROUND_POINT`, used at 1280x720).
    for (output, size) in [
        ((1280, 720), (896, 524)),
        ((1024, 768), (717, 562)),
        ((1280, 800), (896, 587)),
        ((1920, 1080), (1129, 787)),
    ] {
        let (_, _, w, h) = door_graphics_core::initial_window(output, (1129, 787), 0, None);
        assert_eq!((w, h), size, "{output:?}");
        s.configure(w, h);
        let pixels = paint(&s, w, h, false, None, None);
        let search = door_workspace::settings_layout::Layout {
            width: w,
            height: h,
        }
        .search()
        .unwrap();
        assert!(search.y + search.h as i32 <= 48, "{output:?}");
        // The cached-drag exposure probe moves left along this row to stay
        // clear of the moved window's shadow (`SETTINGS_EXPOSED_MIN_X`).
        for x in 60..=75 {
            assert_eq!(pixel(&pixels, w, x, 130), bg, "{output:?} x={x}");
        }
    }
    // shell.rs `caption-close-cancelled`: 14 px left of Close's centre
    // (`window_points::settings_caption`, (1062, 90) at (192, 74)) is the
    // client-drawn title bar of the 896x524 window.
    s.configure(896, 524);
    let pixels = paint(&s, 896, 524, false, None, None);
    assert_eq!(pixel(&pixels, 896, 1062 - 14 - 192, 90 - 74), bg);
}

/// tools/xtask/src/desktop/resolution.rs `verify_modes`: Settings maximized
/// at 1920x1080 (1x, client 1920x1032) and 2560x1600 (2x, 1280x752). Probes
/// are physical in the scenario; here they are divided by the density.
#[test]
fn resolution_scenario_points_follow_the_settings_layout() {
    let startup = startup();
    let light = [250, 247, 244, 255];
    for (w, h, scale) in [(1920u32, 1032u32, 1i32), (1280, 752, 2)] {
        let (pw, ph) = (w as i32 * scale, (h as i32 + 48) * scale);
        let at = |x: i32, y: i32| (x / scale, y / scale);
        let mut s = settings::Settings::review(&startup, Page::Home, false);
        s.configure(w, h);
        // The maximized Home page: right content margin, mid and low.
        let px = paint(&s, w, h, false, None, None);
        for (x, y) in [at(pw - 10, ph / 2), at(pw - 10, ph - 100)] {
            assert_eq!(pixel(&px, w, x, y), light, "{w}: ({x}, {y})");
        }
        // Personalization rail row, then Doors (dark) (gallery tile 0);
        // later Doors (light) (tile 2) restores the default.
        s.click_with(140, 326, None).unwrap();
        assert_eq!(s.current_page(), Page::Personalization);
        s.click_with(660, 170, None).unwrap();
        assert_eq!(s.pending_theme, Some(3), "{w}: Doors (dark)");
        s.pending_theme = None;
        s.appearance_changed(3);
        let px = paint(&s, w, h, true, None, None);
        let (bx, by) = at(pw - 20 * scale, ph - 100);
        assert_eq!(pixel(&px, w, bx, by), [36, 32, 31, 255], "{w}: dark window");
        // Tile windows keep their own theme's mode in both gallery rows
        // (Doors (dark), Glow, Doors (light); Captured Motion, Sunrise,
        // Flow); the applied tile carries the dark-mode accent ring.
        for ((x, y), bgra) in [
            ((692, 203), [40, 37, 36, 255]),
            ((933, 203), [249, 249, 249, 255]),
            ((933, 302), [249, 249, 249, 255]),
            ((692, 302), [40, 37, 36, 255]),
            ((608, 183), [255, 205, 96, 255]),
        ] {
            assert_eq!(pixel(&px, w, x, y), bgra, "{w}: ({x}, {y})");
        }
        s.click_with(900, 170, None).unwrap();
        assert_eq!(s.pending_theme, Some(0), "{w}: Doors (light)");
    }
}

/// New windows open at 70% x 78% of the work area (`initial_window`), so
/// Settings' default window is much smaller than its 1129x787 request on the
/// common desktops. Every page lays out there: the main column stays, the
/// search stays in the title bar (26H2 has none in the rail), and no target
/// escapes the window.
#[test]
fn every_page_lays_out_at_the_default_window_sizes() {
    let startup = startup();
    for output in [(1280, 720), (1024, 768), (1280, 800), (1920, 1080)] {
        let (_, _, w, h) = door_graphics_core::initial_window(output, (1129, 787), 0, None);
        let layout = door_workspace::settings_layout::Layout {
            width: w,
            height: h,
        };
        assert!(layout.has_main(), "{w}x{h}");
        let search = layout.search().unwrap();
        // The centred control keeps 190 px margins for back, the title and
        // the caption buttons.
        assert_eq!(layout.caption_control(), Some((search.x, search.w)));
        assert!(search.x >= 190 && search.x + search.w as i32 <= w as i32 - 190);
        assert_eq!(layout.rail_top(), 144, "{w}x{h}");
        for page in Page::ALL {
            for dark in [false, true] {
                let mut s = settings::Settings::review(&startup, page, dark);
                s.configure(w, h);
                let name = format!(
                    "default-{w}x{h}-{}-{}",
                    page as u32,
                    if dark { "dark" } else { "light" }
                );
                let save = !dark && ((w, h) == (896, 524) || page == Page::Home);
                paint(&s, w, h, dark, None, save.then_some(name.as_str()));
                for y in [60, 200, h as i32 / 2, h as i32 - 8] {
                    assert!(
                        s.target(w as i32 - 1, y).is_none_or(|id| id == 400),
                        "{page:?} {w}x{h} y {y}"
                    );
                }
            }
        }
    }
}

/// Paint with a held pointer press at `at` (hover and pressed).
fn paint_pressed(s: &settings::Settings, width: u32, height: u32, at: (i32, i32)) -> Vec<u8> {
    let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
    let images = ui::ImageCache::default();
    let mut pixels = vec![0; width as usize * height as usize * 4];
    {
        let mut view =
            ui::View::new_scaled(&mut pixels, width, height, &fonts, 1).with_images(&images);
        view.hover = Some(at);
        view.pressed = Some(at);
        s.draw(&mut view);
    }
    pixels
}

/// Windows 11 Home's header buttons: the network and Windows Update status
/// tiles (Doors: Network, Doors Update) beside the device card when wide,
/// wrapped under it at the 896 and 717 px default windows. They hover,
/// press, take keyboard focus first and navigate like any Settings link
/// (`SETTINGS PAGE id=3` / `id=11`). System's header has the update tile.
#[test]
fn home_and_system_header_status_tiles_are_windows_buttons() {
    use door_workspace::settings_catalog::Hero;
    let startup = startup();
    let apps = door_workspace::settings_layout::INCLUDED_APPS.len();
    let (bg, plate, pressed) = (
        [250, 247, 244, 255],
        [242, 238, 235, 255],
        [239, 235, 232, 255],
    );
    assert_eq!((Page::Network as usize, Page::Update as usize), (3, 11));
    for (window, content, wide, home, system) in [
        (
            1129u32,
            797u32,
            true,
            [(659, 122, 220), (887, 122, 220)],
            (887, 122, 220),
        ),
        (
            896,
            564,
            false,
            [(310, 205, 220), (538, 205, 220)],
            (654, 122, 220),
        ),
        (
            717,
            385,
            false,
            [(310, 205, 188), (506, 205, 188)],
            (310, 205, 220),
        ),
    ] {
        let ctx = catalog::Context {
            width: content,
            open: u64::MAX,
            apps,
            keyboards: 1,
            lists: [0; catalog::List::COUNT],
            vpn_up: 0,
            background: 0,
            recent_accents: 1,
        };
        // Geometry: Home's two tiles lead its focus order.
        let l = catalog::layout(Page::Home, &ctx);
        let (hero, r) = l.blocks[0].hero.unwrap();
        assert_eq!(hero, Hero::Home);
        assert_eq!(catalog::status_wrapped(hero, content), !wide, "{window}");
        assert_eq!(r.h, if wide { 82 } else { 154 }, "{window}");
        let tiles = catalog::status_tiles(hero, r, content);
        assert_eq!(tiles.len(), 2);
        for (i, ((page, tile), (x, y, w))) in tiles.iter().zip(home).enumerate() {
            assert_eq!(*page, [Page::Network, Page::Update][i]);
            assert_eq!(
                (tile.x, tile.y, tile.w, tile.h),
                (x, y, w, 60),
                "{window} tile {i}"
            );
            assert_eq!(l.items[i].rect, *tile, "{window}: Tab order");
            assert!(tile.right() <= r.right() && tile.bottom() <= r.bottom());
        }
        let l = catalog::layout(Page::System, &ctx);
        let (hero, r) = l.blocks[0].hero.unwrap();
        assert_eq!(hero, Hero::Device);
        let tiles = catalog::status_tiles(hero, r, content);
        let (page, tile) = tiles[0];
        assert_eq!(page, Page::Update);
        assert_eq!((tile.x, tile.y, tile.w), system, "{window}: System");
        assert_eq!(l.items[0].rect, tile);
        // Painter: idle, hover plate, darker while pressed (light theme).
        for (index, page) in [(0usize, Page::Network), (1, Page::Update)] {
            let tile = catalog::status_tiles(
                Hero::Home,
                catalog::layout(Page::Home, &ctx).blocks[0].hero.unwrap().1,
                content,
            )[index]
                .1;
            let centre = (tile.x + tile.w as i32 / 2, tile.y + tile.h as i32 / 2);
            let probe = (tile.right() - 6, tile.y + 4);
            let mut s = settings::Settings::review(&startup, Page::Home, false);
            s.configure(window, 787);
            let idle = paint(&s, window, 787, false, None, None);
            assert_eq!(pixel(&idle, window, probe.0, probe.1), bg, "{window}");
            let hovered = paint(&s, window, 787, false, Some(centre), None);
            assert_eq!(pixel(&hovered, window, probe.0, probe.1), plate, "{window}");
            let held = paint_pressed(&s, window, 787, centre);
            assert_eq!(pixel(&held, window, probe.0, probe.1), pressed, "{window}");
            // Click navigates like a link.
            s.click_with(centre.0, centre.1, None).unwrap();
            assert_eq!(s.current_page(), page, "{window}");
            // Keyboard: Tab reaches the tile (focus ring), Enter/Space opens it.
            let mut s = settings::Settings::review(&startup, Page::Home, false);
            s.configure(window, 787);
            for _ in 0..=index {
                s.key_with(TAB, None).unwrap();
            }
            let focused = paint(&s, window, 787, false, None, None);
            assert_eq!(
                pixel(&focused, window, tile.x - 3, tile.y + tile.h as i32 / 2),
                [26, 26, 26, 255],
                "{window}: focus ring on tile {index}"
            );
            s.key_with(if index == 0 { RETURN } else { SPACE }, None)
                .unwrap();
            assert_eq!(s.current_page(), page, "{window}: keyboard");
        }
        if window == 896 {
            let s = settings::Settings::review(&startup, Page::Home, false);
            let mut s = s;
            s.configure(window, 524);
            paint(
                &s,
                window,
                524,
                false,
                Some((420, 248)),
                Some("home-status-tiles-896"),
            );
        }
    }
}
/// tools/xtask/src/desktop/administration.rs and settings_reference.rs:
/// the Registry Editor's card buttons and its dialogs' fields, type combo
/// and buttons, at the points `settings_catalog::points` derives (Settings
/// maximized: a 1280x672 or 1920x1032 client at the screen origin, the page
/// at the top; the restored 896x524 window for the second Edit). The states
/// mirror the scenario: HKEY_CURRENT_USER, then
/// HKCU\Software\Door\AdminSettings with the saved Probe value.
#[test]
fn administration_scenario_registry_points_follow_the_page_layout() {
    use catalog::{Command, DialogKind, List, points};
    use settings::dialog::Focus;
    let startup = startup();
    for (width, height) in [(1280u32, 672u32), (1920, 1032)] {
        let mut s = settings::Settings::review(&startup, Page::Registry, false);
        s.configure(width, height);
        s.review_registry(&[], &[("DesktopControls", 4, "0x30")], None);
        let go_to = points::command(Page::Registry, Command::RegistryGoTo, width, height).unwrap();
        let new_value =
            points::command(Page::Registry, Command::RegistryNewValue, width, height).unwrap();
        // Both buttons are on screen without scrolling.
        assert!(go_to.1 < height as i32 - 20 && new_value.1 < height as i32 - 20);
        // Go to a key: the dialog opens on the current path, selected.
        s.click_with(go_to.0, go_to.1, None).unwrap();
        let (kind, focus, fields, _, _) = s.dialog_state().unwrap();
        assert_eq!((kind, focus), (DialogKind::RegistryGoTo, Focus::Field(0)));
        assert_eq!(fields[0], "HKEY_CURRENT_USER");
        let field = points::dialog_field(DialogKind::RegistryGoTo, 0, width, height);
        let create = points::dialog_button(DialogKind::RegistryGoTo, 1, width, height);
        assert_eq!(s.target(field.0, field.1), Some(settings::dialog::FIELD));
        assert_eq!(
            s.target(create.0, create.1),
            Some(settings::dialog::BUTTON + 1)
        );
        // The page behind a dialog never reacts.
        assert_eq!(
            s.target(new_value.0, new_value.1),
            Some(settings::dialog::BACKGROUND)
        );
        paint(
            &s,
            width,
            height,
            false,
            Some(create),
            (width == 1280).then_some("registry-go-to-dialog"),
        );
        // Escape cancels and returns to the page.
        s.key_with(1, None).unwrap();
        assert!(s.dialog_state().is_none());
        // New value: Name has focus; the type list opens on REG_SZ, and
        // Down x kind + Return reaches each scenario type.
        let kind_point = points::dialog_type(DialogKind::RegistryValue, width, height);
        let data = points::dialog_field(DialogKind::RegistryValue, 1, width, height);
        for (downs, kind) in [(3, 4), (0, 1), (1, 2), (2, 3), (4, 7), (5, 11), (6, 0)] {
            s.click_with(new_value.0, new_value.1, None).unwrap();
            let (dk, focus, _, value_kind, _) = s.dialog_state().unwrap();
            assert_eq!(
                (dk, focus, value_kind),
                (DialogKind::RegistryValue, Focus::Field(0), 1)
            );
            for c in "Probe".chars() {
                s.key_with(ch(c), None).unwrap();
            }
            s.click_with(kind_point.0, kind_point.1, None).unwrap();
            for _ in 0..downs {
                s.key_with(DOWN, None).unwrap();
            }
            s.key_with(RETURN, None).unwrap();
            assert_eq!(s.dialog_state().unwrap().3, kind, "{downs} downs");
            assert_eq!(s.target(data.0, data.1), Some(settings::dialog::FIELD + 1));
            s.click_with(data.0, data.1, None).unwrap();
            for c in "12".chars() {
                s.key_with(ch(c), None).unwrap();
            }
            let (_, focus, fields, _, _) = s.dialog_state().unwrap();
            assert_eq!(focus, Focus::Field(1));
            assert_eq!(fields, [String::from("Probe"), String::from("12")]);
            if downs == 3 && width == 1280 {
                paint(
                    &s,
                    width,
                    height,
                    false,
                    None,
                    Some("registry-new-value-dialog"),
                );
            }
            s.key_with(1, None).unwrap();
        }
        // AdminSettings with Probe: the first value's Edit opens its data,
        // selected, in the same dialog.
        let mut lists = [0; List::COUNT];
        lists[List::RegistryValues as usize] = 1;
        s.review_registry(&[], &[("Probe", 4, "0x1234ABCD")], None);
        let first = points::list_edit(
            Page::Registry,
            List::RegistryValues,
            0,
            width,
            height,
            lists,
        )
        .unwrap();
        assert!(
            first.1 < height as i32 - 20,
            "first value Edit is on screen"
        );
        s.click_with(first.0, first.1, None).unwrap();
        let (dk, focus, fields, kind, _) = s.dialog_state().unwrap();
        assert_eq!(
            (dk, focus, kind),
            (DialogKind::RegistryEdit, Focus::Field(1), 4)
        );
        assert_eq!(fields[0], "Probe");
        assert_eq!(s.target(data.0, data.1), Some(settings::dialog::FIELD + 1));
        // Escape cancels; focus returns to Edit, so Return opens it again.
        s.key_with(1, None).unwrap();
        assert!(s.dialog_state().is_none());
        s.key_with(RETURN, None).unwrap();
        assert_eq!(s.dialog_state().unwrap().2[0], "Probe");
        // In the restored 896x524 window the dialog's Cancel is pressable.
        s.configure(896, 524);
        let cancel = points::dialog_button(DialogKind::RegistryEdit, 2, 896, 524);
        assert_eq!(
            s.target(cancel.0, cancel.1),
            Some(settings::dialog::BUTTON + 2)
        );
        let g = catalog::dialog_geometry(DialogKind::RegistryValue, 896, 524);
        assert!(g.rect.y >= 0 && g.rect.bottom() <= 524, "{g:?}");
        paint(
            &s,
            896,
            524,
            false,
            Some(cancel),
            (width == 1280).then_some("registry-restored-cancel"),
        );
        s.click_with(cancel.0, cancel.1, None).unwrap();
        assert!(s.dialog_state().is_none());
    }
}

/// tools/xtask/src/desktop/administration.rs `page`: Esc, Ctrl+F, the query,
/// Return, Esc lands on each unified administration page (the scenario waits
/// for `SETTINGS PAGE id=<id>`), and Alt+Left / Alt+Right walk Registry and
/// Services history.
#[test]
fn administration_scenario_pages_and_history_follow_search() {
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::Personalization, false);
    s.configure(1280, 672);
    for (query, id) in ADMINISTRATION_QUERIES {
        search_page(&mut s, query);
        assert_eq!(s.current_page() as usize, *id, "{query}");
    }
    search_page(&mut s, "registry");
    search_page(&mut s, "services");
    s.key_with(alt(0x14b), None).unwrap();
    assert_eq!(s.current_page(), Page::Registry);
    s.key_with(alt(0x14d), None).unwrap();
    assert_eq!(s.current_page(), Page::Services);
    search_page(&mut s, "personalization");
    assert_eq!(s.current_page(), Page::Personalization);
}
/// The administration scenario's sixteen queries (kept in step with
/// tools/xtask/src/desktop/administration.rs).
const ADMINISTRATION_QUERIES: &[(&str, usize)] = &[
    ("drivers", 18),
    ("disk management", 19),
    ("advanced network settings", 20),
    ("services", 21),
    ("environment", 22),
    ("system configuration", 23),
    ("registry", 17),
    ("group policy", 24),
    ("event viewer", 25),
    ("task scheduler", 26),
    ("your info", 27),
    ("file system", 28),
    ("troubleshoot", 29),
    ("power options", 30),
    ("recovery", 31),
    ("developer tools", 32),
];
/// Legacy names typed into Settings search find their replacement first.
#[test]
fn search_finds_legacy_tools_and_sections_and_highlights_the_target() {
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1129, 787);
    for (query, page, highlight) in [
        ("Device Manager", Page::DevicesDrivers, None),
        ("regedit", Page::Registry, None),
        ("Control Panel", Page::Home, None),
        ("night light", Page::NightLight, None),
        (
            "Credential Manager",
            Page::Accounts,
            Some("Saved credentials"),
        ),
        (
            "Doors Tools",
            Page::SystemAdvanced,
            Some("Administrative tools"),
        ),
    ] {
        search_page(&mut s, query);
        assert_eq!(s.current_page(), page, "{query}");
        assert_eq!(s.highlighted(), highlight, "{query}");
    }
    // The highlighted section is open (an expander) and on screen.
    paint(
        &s,
        1129,
        787,
        false,
        None,
        Some("search-windows-tools-highlight"),
    );
    let layout = catalog::layout(
        Page::SystemAdvanced,
        &catalog::points::context(1129, 787, [0; catalog::List::COUNT]),
    );
    let _ = layout;
}
/// Deep links from the shell (SETTINGS_NAVIGATE): a page, or a page and a
/// section, as `door_settings_map::navigate_arg` encodes them.
#[test]
fn deep_links_open_the_page_and_highlight_the_section() {
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1129, 787);
    for command in [
        "sysdm.cpl",
        "control sysdm.cpl,,3",
        "ncpa.cpl",
        "devmgmt.msc",
        "regedit",
        "powercfg.cpl",
        "rundll32 sysdm.cpl,EditEnvironmentVariables",
        "ms-settings:startupapps",
        "control /name Microsoft.CredentialManager",
    ] {
        let (page, anchor) = door_settings_map::resolve(command).unwrap();
        s.navigate_requested(door_settings_map::navigate_arg(page, anchor))
            .unwrap();
        assert_eq!(s.current_page(), page, "{command}");
        let expected = anchor.map(|a| door_settings_map::section(page, a).unwrap().title);
        assert_eq!(s.highlighted(), expected, "{command}");
    }
    // Malformed arguments are rejected.
    assert!(s.navigate_requested(Page::COUNT as u32).is_err());
    assert!(s.navigate_requested(1 | (999 << 16)).is_err());
}
/// Every deep-link section names a block or row its page really shows.
#[test]
fn every_section_anchor_exists_on_its_page() {
    for section in door_settings_map::SECTIONS {
        let mut ctx = catalog::points::context(1129, 787, [0; catalog::List::COUNT]);
        ctx.open = u64::MAX;
        let layout = catalog::layout(section.page, &ctx);
        assert!(
            catalog::section_rect(&layout, section.title).is_some(),
            "{} on {:?}: no '{}'",
            section.anchor,
            section.page,
            section.title
        );
    }
}
/// Every card has a title, and every control either acts or says it is
/// currently unavailable (the painter labels disabled buttons and controls
/// without a provider).
#[test]
fn administration_cards_describe_themselves_and_have_no_dead_controls() {
    use catalog::{Block, Control};
    for page in Page::ALL {
        for block in catalog::blocks_for(page) {
            let rows: Vec<catalog::Row> = match block {
                Block::Card(row) => vec![*row],
                Block::Group(g) => g.rows.to_vec(),
                Block::Expander(e) => {
                    let mut v = vec![e.header];
                    v.extend_from_slice(e.rows);
                    v
                }
                _ => vec![],
            };
            for row in rows {
                assert!(!row.title.is_empty(), "{page:?}");
                if let Control::Button(label) = row.control {
                    assert!(!label.is_empty(), "{page:?}: {}", row.title);
                }
            }
        }
    }
}

// ---- W7: contrast themes, text size and display scale -------------------

use door_service_protocol::desktop::{ContrastTheme, SystemColors};
use door_workspace::palette::Palette;

/// Paint `s` with a contrast theme on (as the session's palette would be).
fn paint_contrast(
    s: &settings::Settings,
    width: u32,
    height: u32,
    theme: ContrastTheme,
    name: Option<&str>,
) -> Vec<u8> {
    let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
    let images = ui::ImageCache::default();
    let mut pixels = vec![0; width as usize * height as usize * 4];
    {
        let mut view =
            ui::View::new_scaled(&mut pixels, width, height, &fonts, 1).with_images(&images);
        view.palette = Palette {
            dark: false,
            contrast: Some(theme),
        };
        s.draw(&mut view);
    }
    if let Some(name) = name {
        let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
        ppm.extend(pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]));
        std::fs::write(directory().join(format!("{name}.ppm")), ppm).unwrap();
    }
    pixels
}
/// WCAG relative luminance of an RGB colour.
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
fn bgr(p: [u8; 4]) -> [u8; 3] {
    [p[2], p[1], p[0]]
}
/// Whether `p` is (an anti-aliased blend of) two of the theme's colours.
/// Text is ClearType-filtered, so each channel blends on its own: every
/// channel lies between the two colours' channels.
fn system_blend(s: SystemColors, p: [u8; 3]) -> bool {
    let colours = [
        s.window,
        s.window_text,
        s.highlight,
        s.highlight_text,
        s.button_face,
        s.button_text,
        s.gray_text,
        s.hotlight,
    ];
    colours.iter().any(|a| {
        colours.iter().any(|b| {
            (0..3).all(|i| {
                let (lo, hi) = (a[i].min(b[i]), a[i].max(b[i]));
                p[i] >= lo.saturating_sub(3) && p[i] <= hi.saturating_add(3)
            })
        })
    })
}
/// Text ink of a frame painted in contrast theme `s`: for every pixel of a
/// text role, its background is the most common surface role around it.
/// Returns (minimum ratio of text against its background, stray pixels that
/// are no blend of system colours, text pixels examined).
fn contrast_audit(
    s: SystemColors,
    pixels: &[u8],
    width: u32,
    height: u32,
    pictures: &[(i32, i32, u32, u32)],
) -> (f64, usize, usize) {
    let at = |x: i32, y: i32| bgr(pixel(pixels, width, x, y));
    // Theme and background thumbnails keep their pictures (with the
    // selection ring around them), as in Windows.
    let picture = |x: i32, y: i32| {
        pictures.iter().any(|&(px, py, w, h)| {
            x >= px - 3 && y >= py - 3 && x < px + w as i32 + 3 && y < py + h as i32 + 3
        })
    };
    let surfaces = [s.window, s.highlight, s.button_face];
    let inks = [s.window_text, s.highlight_text, s.button_text, s.hotlight];
    let (mut min, mut strays, mut examined) = (f64::MAX, 0, 0);
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let p = at(x, y);
            if picture(x, y) {
                continue;
            }
            if !system_blend(s, p) {
                strays += 1;
                continue;
            }
            // An ink colour between Window and WindowText (Night sky's
            // HighlightText 2B2B2B) may be an anti-aliased WindowText edge.
            let edge = p != s.window_text
                && (0..3).all(|i| {
                    p[i] >= s.window[i].min(s.window_text[i])
                        && p[i] <= s.window[i].max(s.window_text[i])
                });
            if !inks.contains(&p) || surfaces.contains(&p) || edge {
                continue;
            }
            // Glyph strokes are short: skip long horizontal/vertical runs
            // (WindowText borders and dividers).
            let run = |dx: i32, dy: i32| {
                (1..12)
                    .take_while(|k| {
                        let (u, v) = (x + dx * k, y + dy * k);
                        u >= 0 && v >= 0 && u < width as i32 && v < height as i32 && at(u, v) == p
                    })
                    .count()
            };
            if run(1, 0) + run(-1, 0) >= 10 || run(0, 1) + run(0, -1) >= 10 {
                continue;
            }
            let mut counts = [0usize; 3];
            for v in (y - 6).max(0)..(y + 7).min(height as i32) {
                for u in (x - 6).max(0)..(x + 7).min(width as i32) {
                    if let Some(i) = surfaces.iter().position(|c| *c == at(u, v)) {
                        counts[i] += 1;
                    }
                }
            }
            let (i, n) = counts.iter().enumerate().max_by_key(|(_, n)| **n).unwrap();
            if *n == 0 {
                continue;
            }
            examined += 1;
            min = min.min(ratio(p, surfaces[i]));
        }
    }
    (min, strays, examined)
}
const W7_PAGES: [Page; 10] = [
    Page::Home,
    Page::System,
    Page::Display,
    Page::Personalization,
    Page::Colors,
    Page::Accessibility,
    Page::ContrastThemes,
    Page::TextSize,
    Page::Network,
    Page::About,
];

#[test]
fn contrast_themes_paint_settings_in_system_colours_with_seven_to_one_text() {
    let startup = startup();
    for theme in ContrastTheme::ALL {
        let colours = theme.colors();
        for page in W7_PAGES {
            let mut s = settings::Settings::review(&startup, page, false);
            s.configure(1129, 787);
            let name = format!("contrast-{theme:?}-{page:?}").to_lowercase();
            let pixels = paint_contrast(&s, 1129, 787, theme, Some(&name));
            let pictures = s.picture_rects();
            let (min, strays, examined) = contrast_audit(colours, &pixels, 1129, 787, &pictures);
            eprintln!("{name}: min {min:.2} strays {strays} text {examined}");
            assert!(examined > 200, "{name}: text found");
            // Windows contrast themes guarantee 7:1 text (AAA) except two
            // Desert pairs, by Windows' own colours: selection 6.9988 and
            // hyperlink 1C5E75 on window FFFAEF 6.939.
            assert!(min >= 6.935, "{name}: text contrast {min:.3}");
            // Everything painted maps onto the theme: category icons become
            // monochrome glyphs and desktop previews the solid Window
            // desktop. The remaining strays are colour row artwork (the
            // update and status icons), well under 0.5% of the frame.
            assert!(strays < 1129 * 787 / 200, "{name}: {strays} stray pixels");
        }
    }
}
#[test]
fn contrast_themes_page_picks_applies_and_follows_the_shortcut() {
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::ContrastThemes, false);
    s.configure(1129, 787);
    paint(
        &s,
        1129,
        787,
        false,
        None,
        Some("contrast-themes-page-light"),
    );
    // Tab: the theme list. Arrows move the pick only; nothing applies.
    s.key_with(TAB, None).unwrap();
    s.key_with(DOWN, None).unwrap();
    assert_eq!(bits(&s), 48, "the pick waits for Apply");
    // Tab: Apply turns Aquatic on and keeps the other controls.
    s.key_with(TAB, None).unwrap();
    s.key_with(SPACE, None).unwrap();
    let on = Controls::from_bits(bits(&s)).unwrap();
    assert_eq!(on.contrast(), Some(ContrastTheme::Aquatic));
    assert_eq!(on.with_contrast(None).bits() & !(3 << 22), 48);
    paint_contrast(
        &s,
        1129,
        787,
        ContrastTheme::Aquatic,
        Some("contrast-themes-aquatic"),
    );
    // From Aquatic: the fourth theme down the list is Night sky.
    let mut s = settings::Settings::review(&startup, Page::ContrastThemes, false);
    s.configure(1129, 787);
    s.review_state(0, on.bits());
    s.key_with(TAB, None).unwrap();
    for _ in 0..4 {
        s.key_with(DOWN, None).unwrap();
    }
    s.key_with(TAB, None).unwrap();
    s.key_with(SPACE, None).unwrap();
    let night = Controls::from_bits(bits(&s)).unwrap();
    assert_eq!(night.contrast(), Some(ContrastTheme::NightSky));
    // Left Alt + Left Shift + Print Screen in the shell toggles it off and
    // arrives as CONTROLS_CHANGED: the list follows (None).
    s.pending_controls = None;
    let off = night.toggle_contrast();
    assert_eq!(off.contrast(), None);
    s.controls_changed(off);
    assert_eq!(Controls::from_bits(bits(&s)).unwrap().contrast(), None);
    assert_eq!(
        off.toggle_contrast().contrast(),
        Some(ContrastTheme::NightSky)
    );
}

#[test]
fn text_size_slider_steps_drags_and_applies() {
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::TextSize, false);
    s.configure(1129, 787);
    assert_eq!(s.text_pick(), 100);
    // Tab: the slider. Arrows 1%, Page Up 10%, End 225%, Home 100%.
    s.key_with(TAB, None).unwrap();
    s.key_with(0x14d, None).unwrap();
    assert_eq!(s.text_pick(), 101);
    s.key_with(0x149, None).unwrap();
    assert_eq!(s.text_pick(), 111);
    s.key_with(END, None).unwrap();
    assert_eq!(s.text_pick(), 225);
    paint(&s, 1129, 787, false, None, Some("text-size-225-light"));
    s.key_with(0x147, None).unwrap();
    assert_eq!(s.text_pick(), 100);
    assert_eq!(bits(&s), 48, "the slider waits for Apply");
    // Pointer: press on the slider, drag past either end.
    let mut found = None;
    'find: for y in (120..600).step_by(4) {
        for x in (330..1090).step_by(8) {
            if s.pointer_down(x, y) {
                found = Some((x, y));
                break 'find;
            }
        }
    }
    let (x, y) = found.expect("slider under the pointer");
    s.pointer_motion(x + 2000, y);
    s.cancel_drag();
    assert_eq!(s.text_pick(), 225);
    s.pointer_down(x, y);
    s.pointer_motion(x - 2000, y);
    s.cancel_drag();
    assert_eq!(s.text_pick(), 100);
    // The slider keeps focus: 150%, then Apply.
    for _ in 0..5 {
        s.key_with(0x149, None).unwrap();
    }
    assert_eq!(s.text_pick(), 150);
    s.key_with(TAB, None).unwrap();
    s.key_with(SPACE, None).unwrap();
    assert_eq!(Controls::from_bits(bits(&s)).unwrap().text_percent(), 150);
    paint(&s, 1129, 787, true, None, Some("text-size-150-dark"));
}

#[test]
fn display_scale_lists_offered_scales_and_persists_only_accepted_ones() {
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::Display, false);
    s.configure(1129, 787);
    // The window server's report: 150% recommended; 250% not offered.
    s.scale_info(150, 150, 0b1011_1111);
    assert_eq!(
        s.scale_labels(),
        [
            "100%",
            "125%",
            "150% (Recommended)",
            "175%",
            "200%",
            "225%",
            "300%"
        ]
    );
    // Focus the Scale combo (the review opens every expander, so Tab past
    // Night light's strength), then the arrows step it.
    for _ in 0..8 {
        s.key_with(TAB, None).unwrap();
        if s.focused_action()
            == Some(door_workspace::settings_catalog::Action::Combo(
                door_workspace::settings_catalog::Choice::Scale,
            ))
        {
            break;
        }
    }
    for _ in 0..12 {
        if s.pending_scale.is_some() {
            break;
        }
        s.key_with(DOWN, None).unwrap();
    }
    // Nothing is stored until the window server accepts the scale.
    assert_eq!(s.pending_scale, Some(175));
    assert_eq!(s.scale_state(), (175, false));
    s.scale_refused();
    assert_eq!(
        (s.pending_scale, s.scale_state()),
        (None, (0, false)),
        "refused"
    );
    s.key_with(DOWN, None).unwrap();
    assert_eq!(s.pending_scale, Some(175));
    s.scale_applied(175, None).unwrap();
    assert_eq!(
        s.scale_state(),
        (175, false),
        "applied live: no sign-out note"
    );
    // The recommended scale stores 0 (automatic).
    s.key_with(UP, None).unwrap();
    assert_eq!(s.pending_scale, Some(0));
    s.scale_applied(150, None).unwrap();
    assert_eq!(s.scale_state(), (0, false));
    // 225% steps to 300%: 250% is not offered.
    for (key, percent) in [(DOWN, 175), (DOWN, 200), (DOWN, 225), (DOWN, 300)] {
        s.key_with(key, None).unwrap();
        assert_eq!(s.pending_scale, Some(percent));
        s.scale_applied(percent, None).unwrap();
    }
    // The open list shows the offered scales with the recommended mark.
    s.key_with(RETURN, None).unwrap();
    assert!(s.menu_open());
    paint(&s, 1129, 787, false, None, Some("display-scale-list"));
    keys(&mut s, &[ESC]);
    // A window server without live scale changes: kept for the next
    // sign-in, and the row says so.
    s.key_with(UP, None).unwrap();
    s.scale_deferred(None).unwrap();
    assert_eq!(s.scale_state(), (225, true));
    paint(&s, 1129, 787, false, None, Some("display-scale-sign-out"));
}

// ---------------------------------------------------------------------------
// Key sequences and page ids the P12 acceptance suites send to Settings
// (tools/xtask/src/desktop/{text_editing,display_scale,motion_timing}.rs).
// The suites wait for `SETTINGS PAGE id=` with these ids.
// ---------------------------------------------------------------------------
const PGUP: u32 = 0x149;
const HOME: u32 = 0x147;
const RIGHT: u32 = 0x14d;
#[test]
fn acceptance_suite_settings_key_paths() {
    let startup = startup();
    for (page, id) in [
        (Page::Display, 13),
        (Page::VisualEffects, 43),
        (Page::TextSize, 47),
        (Page::Typing, 48),
    ] {
        assert_eq!(page as usize, id, "{page:?}");
    }
    // TEXT-01: Typing, Tab to Add a keyboard, Return, Down x5 (Microsoft
    // Pinyin, the sixth addable method), Return: [US, Pinyin] = 0x71.
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1280, 800);
    search_page(&mut s, "typing");
    assert_eq!(s.current_page(), Page::Typing);
    keys(&mut s, &[TAB, RETURN]);
    assert!(s.menu_open());
    keys(&mut s, &[DOWN, DOWN, DOWN, DOWN, DOWN, RETURN]);
    assert_eq!(s.pending_input, Some(0x71));
    // DISPLAY-01 on 2560x1600 (recommended and initial 200%; 100..=200%
    // offered): four Tabs pass the Brightness slider, Night light (its
    // card, its switch) and HDR, the fifth focuses Scale; Up steps down one
    // scale, Down up one.
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.display(2560, 1600, 200);
    s.configure(1100, 700);
    search_page(&mut s, "display");
    assert_eq!(s.current_page(), Page::Display);
    s.scale_info(200, 200, 0b1_1111);
    for _ in 0..5 {
        s.key_with(TAB, None).unwrap();
    }
    assert_eq!(
        s.focused_action(),
        Some(door_workspace::settings_catalog::Action::Combo(
            door_workspace::settings_catalog::Choice::Scale
        ))
    );
    let mut seen = Vec::new();
    for (key, stored, effective) in [
        (UP, 175, 175),
        (UP, 150, 150),
        (UP, 125, 125),
        (UP, 100, 100),
        (DOWN, 125, 125),
        (DOWN, 150, 150),
        (DOWN, 175, 175),
        (DOWN, 0, 200),
        (UP, 175, 175),
        (UP, 150, 150),
    ] {
        s.key_with(key, None).unwrap();
        assert_eq!(s.pending_scale, Some(stored), "after {seen:?}");
        // The window server applies it; SCALE_CHANGED and CONFIGURED follow.
        s.scale_applied(effective, None).unwrap();
        s.display(2560, 1600, effective);
        s.scale_info(effective, 200, 0b1_1111);
        s.configure(1100 * 200 / effective, 700 * 200 / effective);
        seen.push(effective);
    }
    // Text size: Tab to the slider, Page Up x2 and Right x5 (125%), Tab to
    // Apply, Space; back: Shift+Tab, Home (100%), Tab, Space.
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1280, 800);
    search_page(&mut s, "text size");
    assert_eq!(s.current_page(), Page::TextSize);
    keys(
        &mut s,
        &[TAB, PGUP, PGUP, RIGHT, RIGHT, RIGHT, RIGHT, RIGHT],
    );
    assert_eq!(s.text_pick(), 125);
    keys(&mut s, &[TAB, SPACE]);
    assert_eq!(bits(&s), 48 | 25 << Controls::TEXT_SHIFT);
    keys(&mut s, &[shift(TAB), HOME, TAB, SPACE]);
    assert_eq!(bits(&s), 48);
    // MOTION-01: Visual effects, Tab x2 to Animation effects, Space sets
    // REDUCE_MOTION (4144); the same path from a fresh search restores 48.
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1280, 800);
    search_page(&mut s, "visual effects");
    assert_eq!(s.current_page(), Page::VisualEffects);
    keys(&mut s, &[TAB, TAB, SPACE]);
    assert_eq!(bits(&s), 48 | Controls::REDUCE_MOTION);
    search_page(&mut s, "visual effects");
    keys(&mut s, &[TAB, TAB, SPACE]);
    assert_eq!(bits(&s), 48);
}

/// tools/xtask/src/desktop/p14_settings.rs: Environment variables by search,
/// Tab to Add a variable, Return opens the dialog with Name focused, the
/// name, Tab to Value, the value; Return presses Save (the guest saves; a
/// host review has no services and keeps the dialog with its reason). The
/// pointer path System > Storage > Advanced storage settings > Disks &
/// volumes reaches page 19.
#[test]
fn p14_settings_scenario_keyboard_and_pointer_paths() {
    use catalog::{Action, DialogKind, List, points};
    use settings::dialog::Focus;
    let startup = startup();
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1280, 672);
    search_page(&mut s, "environment");
    assert_eq!(s.current_page(), Page::Environment);
    s.key_with(TAB, None).unwrap();
    s.key_with(RETURN, None).unwrap();
    let (kind, focus, ..) = s.dialog_state().unwrap();
    assert_eq!((kind, focus), (DialogKind::Variable, Focus::Field(0)));
    for c in "DOOR_P14".chars() {
        s.key_with(ch(c), None).unwrap();
    }
    s.key_with(TAB, None).unwrap();
    for c in "persisted".chars() {
        s.key_with(ch(c), None).unwrap();
    }
    let (_, focus, fields, ..) = s.dialog_state().unwrap();
    assert_eq!(focus, Focus::Field(1));
    assert_eq!(
        fields,
        [String::from("DOOR_P14"), String::from("persisted")]
    );
    s.key_with(RETURN, None).unwrap();
    assert!(
        !s.dialog_state().unwrap().4.is_empty(),
        "no services on the host"
    );
    s.key_with(ESC, None).unwrap();
    assert!(s.dialog_state().is_none());
    // Pointer: the rail's System row, the Storage card, the expander, the row.
    let mut s = settings::Settings::review(&startup, Page::Home, false);
    s.configure(1280, 672);
    s.review_state(0, 48);
    let rail = door_workspace::settings_navigation::Rail::new(
        door_workspace::settings_layout::Layout {
            width: 1280,
            height: 672,
        },
        0,
    );
    let system = (
        door_workspace::settings_layout::Layout::NAV_X + 100,
        rail.row(1) + door_workspace::settings_layout::Layout::nav_row() / 2,
    );
    s.click_with(system.0, system.1, None).unwrap();
    assert_eq!(s.current_page(), Page::System);
    let lists = [0; List::COUNT];
    let storage = points::item(Page::System, 1280, 672, lists, |i| {
        i.action == Action::Navigate(Page::Storage)
    })
    .unwrap();
    s.click_with(storage.0, storage.1, None).unwrap();
    assert_eq!(s.current_page(), Page::Storage);
    let expander = points::item(Page::Storage, 1280, 672, lists, |i| {
        i.action == Action::Expand(1)
    })
    .unwrap();
    s.click_with(expander.0, expander.1, None).unwrap();
    let mut ctx = points::context(1280, 672, lists);
    ctx.open = 1 << 1;
    let disks = catalog::layout(Page::Storage, &ctx)
        .items
        .iter()
        .find(|i| i.action == Action::Navigate(Page::DiskManagement))
        .map(|i| (i.rect.x + i.rect.w as i32 / 2, i.rect.center_y()))
        .unwrap();
    s.click_with(disks.0, disks.1, None).unwrap();
    assert_eq!(s.current_page(), Page::DiskManagement);
    s.key_with(alt(0x14b), None).unwrap();
    assert_eq!(s.current_page(), Page::Storage);
}

/// Background (26H2): the rows follow "Personalize your background".
/// Picture: Recent images, Choose a photo, the fit; Solid color: the
/// swatches and Custom colors; Slideshow: album, interval, shuffle, battery
/// and the fit. A swatch makes the background that solid color.
#[test]
fn background_rows_follow_the_background_kind() {
    use catalog::{Action, Choice};
    use door_service_protocol::preferences::{self as p, Background};
    let startup = startup();
    let titles = |kind: u32| {
        let mut ctx = catalog::points::context(1129, 787, [0; catalog::List::COUNT]);
        ctx.open = u64::MAX;
        ctx.background = kind;
        catalog::layout(Page::Background, &ctx)
            .blocks
            .iter()
            .flat_map(|b| b.rows.iter().map(|(row, _)| row.title))
            .collect::<alloc::vec::Vec<_>>()
    };
    assert_eq!(
        titles(0),
        [
            "Choose a photo",
            "Choose a fit for your desktop image",
            "Contrast themes"
        ]
    );
    assert_eq!(titles(1), ["Custom colors", "Contrast themes"]);
    assert_eq!(
        titles(2),
        [
            "Choose a picture album for a slideshow",
            "Change picture every",
            "Shuffle the picture order",
            "Let slideshow run even if I'm on battery power",
            "Choose a fit for your desktop image",
            "Contrast themes",
        ]
    );
    for dark in [true, false] {
        let mode = if dark { "dark" } else { "light" };
        let mut s = settings::Settings::review(&startup, Page::Background, dark);
        s.configure(1129, 787);
        paint(
            &s,
            1129,
            787,
            dark,
            None,
            Some(&format!("background-picture-{mode}")),
        );
        let combo = catalog::points::item(
            Page::Background,
            1129,
            787,
            [0; catalog::List::COUNT],
            |i| i.action == Action::Combo(Choice::BackgroundKind),
        )
        .unwrap();
        // The list puts the current kind on the field; Solid color is next.
        s.click_with(combo.0, combo.1, None).unwrap();
        assert!(s.menu_open());
        s.click_with(combo.0, combo.1 + 36, None).unwrap();
        let word = |s: &settings::Settings| {
            s.pending_prefs
                .iter()
                .rev()
                .find(|(id, _)| *id == p::BACKGROUND)
                .and_then(|(_, bits)| Background::from_bits(*bits))
                .unwrap()
        };
        assert_eq!(word(&s).kind, 1);
        let mut ctx = catalog::points::context(1129, 787, [0; catalog::List::COUNT]);
        ctx.open = u64::MAX;
        ctx.background = 1;
        let swatch = catalog::layout(Page::Background, &ctx)
            .items
            .iter()
            .find(|i| i.action == Action::SolidColor(2))
            .map(|i| (i.rect.x + i.rect.w as i32 / 2, i.rect.center_y()))
            .unwrap();
        s.click_with(swatch.0, swatch.1, None).unwrap();
        assert_eq!((word(&s).kind, word(&s).color), (1, 0xDCDCDC));
        let pixels = paint(
            &s,
            1129,
            787,
            dark,
            None,
            Some(&format!("background-solid-{mode}")),
        );
        // The chosen swatch keeps its exact color and gets the text-colored
        // ring (no dark-mode surface remapping of either).
        assert_eq!(
            pixel(&pixels, 1129, swatch.0, swatch.1),
            [220, 220, 220, 255]
        );
        let ring = pixel(&pixels, 1129, swatch.0 - 22, swatch.1);
        assert!(
            if dark { ring[0] > 200 } else { ring[0] < 60 },
            "{mode} ring {ring:?}"
        );
        s.click_with(combo.0, combo.1, None).unwrap();
        s.click_with(combo.0, combo.1 + 36, None).unwrap();
        assert_eq!(word(&s).kind, 2);
        paint(
            &s,
            1129,
            787,
            dark,
            None,
            Some(&format!("background-slideshow-{mode}")),
        );
    }
}
/// Over Mica (owner, 2026-11-12) Settings clears its background so the
/// compositor's Mica shows, paints its cards as translucent white (a dark
/// card is white at about 5 %), and keeps its text opaque.
#[test]
fn settings_over_mica_is_translucent_with_opaque_text() {
    let startup = startup();
    let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
    let images = ui::ImageCache::default();
    for dark in [true, false] {
        let s = settings::Settings::review(&startup, Page::System, dark);
        let mut s = s;
        s.configure(1129, 787);
        let mut pixels = vec![0; 1129 * 787 * 4];
        {
            let mut view =
                ui::View::new_scaled(&mut pixels, 1129, 787, &fonts, 1).with_images(&images);
            view.palette.dark = dark;
            view.materials = true;
            s.draw(&mut view);
        }
        // Review: the frame over a flat Mica sample (the owner's dark
        // Settings over dark Bloom, RGB 28,32,41; light F3F3F3-ish).
        let mica: [u8; 3] = if dark { [41, 32, 28] } else { [246, 243, 241] };
        let mut ppm = b"P6\n1129 787\n255\n".to_vec();
        ppm.extend(pixels.chunks_exact(4).flat_map(|p| {
            let a = u32::from(p[3]);
            [2, 1, 0]
                .map(|c| (u32::from(p[c]) + u32::from(mica[c]) * (255 - a) / 255).min(255) as u8)
        }));
        let name = if dark {
            "mica-system-dark"
        } else {
            "mica-system-light"
        };
        std::fs::write(directory().join(format!("{name}.ppm")), ppm).unwrap();
        // Premultiplied throughout.
        assert!(
            pixels
                .chunks_exact(4)
                .all(|p| p[..3].iter().all(|c| *c <= p[3]))
        );
        // The page margin right of the cards is clear, for Mica.
        assert_eq!(pixel(&pixels, 1129, 1120, 700), [0, 0, 0, 0]);
        // The search box interior is exactly its control veil (fill once,
        // border as a ring: no stacking on the border fill).
        let sb = door_workspace::settings_layout::Layout {
            width: 1129,
            height: 787,
        }
        .search()
        .unwrap();
        let pick = |pair: ([u8; 4], [u8; 4])| if dark { pair.1 } else { pair.0 };
        let veil = door_workspace::palette::mica_overlay(
            pick(door_workspace::palette::SETTINGS_CONTROL),
            pick(door_workspace::palette::SETTINGS_BG),
        );
        assert_eq!(
            pixel(
                &pixels,
                1129,
                sb.x + sb.w as i32 * 3 / 4,
                sb.y + sb.h as i32 / 2
            ),
            veil,
            "{dark}: search box"
        );
        // A card centre is a light, mostly transparent veil.
        let card = pixel(&pixels, 1129, 900, catalog::TOP + 140);
        assert!(card[3] > 0 && card[3] < 200, "{dark}: card {card:?}");
        if dark {
            assert!((8..=20).contains(&card[3]), "dark card {card:?}");
        }
        // Text (the page heading) has full coverage somewhere.
        let heading = (310..500)
            .flat_map(|x| (60..92).map(move |y| (x, y)))
            .filter(|&(x, y)| pixel(&pixels, 1129, x, y)[3] == 255)
            .count();
        assert!(heading > 30, "{dark}: heading ink {heading}");
    }
}
/// 26H2 page entrance (motion.page_enter): the new page rises 140 epx on
/// (0.1,0.9,0.2,1) over 300 ms; Reduce motion lands it at once.
#[test]
fn page_entrance_rises_140_epx_over_300_ms() {
    let startup = startup();
    for reduce in [false, true] {
        let mut s = settings::Settings::review(&startup, Page::Home, true);
        s.configure(1129, 787);
        s.enable_page_motion();
        if reduce {
            s.review_state(1, 48 | Controls::REDUCE_MOTION);
        }
        s.tick(1000);
        s.click_with(100, 144 + 40 + 10, None).unwrap();
        assert_eq!(s.current_page(), Page::System);
        if reduce {
            assert_eq!(s.entrance_offset_for_test(), 0, "Reduce motion");
            continue;
        }
        assert_eq!(s.entrance_offset_for_test(), 140);
        assert!(s.tick(1003), "repaints while the page rises");
        let mid = s.entrance_offset_for_test();
        assert!((40..80).contains(&mid), "30 ms: {mid}");
        s.tick(1030);
        assert_eq!(s.entrance_offset_for_test(), 0);
        assert!(!s.tick(1031), "settled");
    }
}
