//! Accessibility > Text size (P12-TEXT-SIZE) on the shared workspace
//! controls (classic menus, message dialogs) and Settings (cards, rail,
//! heading, Advanced pages): rows grow by their text's line box, the grown
//! text fits, and semantics bounds equal the hit-test layout.
//!
//! The applied text size is process-global and this binary compiles the
//! Settings module's own unit tests (which assume 100%), so each check
//! re-runs this binary filtered to itself in a child process (`isolated`).
#![allow(dead_code, unused_imports)]
extern crate alloc;
use alloc::{format, string::String};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_client::{Error, Storage};
use door_service_protocol::Code;
use door_workspace::menu;
use door_workspace::palette::set_text_percent;
use door_workspace::semantics::{Role, Semantics};
use door_workspace::settings_layout::{Layout, Page};
use std::sync::Mutex;
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/tokens.rs"]
mod tokens;
#[path = "../src/ui.rs"]
mod ui;

static LOCK: Mutex<()> = Mutex::new(());
const CHILD: &str = "DOOR_TEXT_SIZE_CHILD";

fn isolated(name: &str, body: impl FnOnce()) {
    if std::env::var_os(CHILD).is_some() {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        body();
        set_text_percent(100);
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{name} failed in its child process:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn startup() -> door_abi::platform::StartupV2 {
    use door_abi::{Inherited, Startup, platform::Identity};
    door_abi::platform::StartupV2 {
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

struct Node {
    target: Option<u32>,
    role: Role,
    name: String,
    r: (i32, i32, u32, u32),
}
fn collect(s: Semantics) -> Vec<Node> {
    let built = s.finish(1, 1, "review");
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

#[test]
fn classic_menu_rows_grow_and_semantics_match_hit_testing() {
    isolated(
        "classic_menu_rows_grow_and_semantics_match_hit_testing",
        || {
            for (display, text) in [(100, 225), (150, 150)] {
                set_text_percent(text);
                let fonts = FontAtlas::windows_reference_text(display, text).unwrap();
                let row = door_workspace::menu::Menu::row_height();
                assert_eq!(row, 32 + fonts.line_height(TextStyle::Regular12) - 18);
                let labels = ["Save  Ctrl+S", "Save as", "Reopen", "Rename", "Close"];
                let mut m = door_workspace::menu::Menu::default();
                m.show(40, 60, 1280, 900, labels.len());
                let mut s = Semantics::new(1280, 900);
                door_workspace::semantics::popup_menu(&mut s, &m, &labels, "File");
                let items: Vec<_> = collect(s)
                    .into_iter()
                    .filter(|n| n.role == Role::MenuItem)
                    .collect();
                assert_eq!(items.len(), labels.len());
                for (i, n) in items.iter().enumerate() {
                    assert_eq!(n.r.3, row);
                    let (x, y) = (n.r.0 + n.r.2 as i32 / 2, n.r.1 + n.r.3 as i32 / 2);
                    assert_eq!(m.target(x, y, labels.len()), n.target, "{text}% row {i}");
                    assert_eq!(m.target(x, n.r.1, labels.len()), Some(1000 + i as u32));
                    assert_eq!(
                        m.target(x, n.r.1 + row as i32 - 1, labels.len()),
                        Some(1000 + i as u32)
                    );
                    // The grown label's ink sits inside its row.
                    let ink = fonts.ink_metrics(TextStyle::Regular12);
                    assert!(
                        ink.height + 4 <= row,
                        "{text}%: ink {} in {row}",
                        ink.height
                    );
                }
            }
        },
    );
}

#[test]
fn message_dialog_grows_its_text_area_and_buttons() {
    isolated("message_dialog_grows_its_text_area_and_buttons", || {
        set_text_percent(100);
        let base = door_workspace::dialog::Layout::new(1280, 900, 2);
        set_text_percent(225);
        let fonts = FontAtlas::windows_reference_text(100, 225).unwrap();
        let l = door_workspace::dialog::Layout::new(1280, 900, 2);
        assert_eq!(l.panel.width, base.panel.width);
        for b in &l.buttons[..2] {
            assert_eq!(b.height, 32 + fonts.line_height(TextStyle::Regular14) - 20);
            assert!(b.y as u32 + b.height <= l.panel.y as u32 + l.panel.height);
        }
        // Title plus five body lines, as at 100%, at the grown line boxes.
        let need = fonts.line_height(TextStyle::Semibold20)
            + 12
            + 4 * fonts.line_height(TextStyle::Regular14);
        assert!(l.text.height >= need, "{} < {need}", l.text.height);
        assert!(l.text.y as u32 + l.text.height <= l.buttons[0].y as u32);
        // A short output still contains everything.
        let small = door_workspace::dialog::Layout::new(640, 360, 3);
        assert!(small.panel.y >= 0 && small.panel.y as u32 + small.panel.height <= 360);
    });
}

#[test]
fn settings_cards_rail_and_heading_grow_without_clipping_and_match_semantics() {
    isolated(
        "settings_cards_rail_and_heading_grow_without_clipping_and_match_semantics",
        || {
            let startup = startup();
            for (display, text) in [(100, 225), (150, 150)] {
                set_text_percent(text);
                let fonts = FontAtlas::windows_reference_text(display, text).unwrap();
                let line14 = fonts.line_height(TextStyle::Regular14);
                let line12 = fonts.line_height(TextStyle::Regular12);
                // A card keeps its padding around the grown title and description.
                let card = door_workspace::settings_catalog::card();
                assert_eq!(
                    card,
                    door_workspace::settings_catalog::CARD + (line14 - 20) + (line12 - 18)
                );
                assert!(card >= line14 + line12 + 24);
                assert_eq!(Layout::nav_row(), 36 + line14 as i32 - 20);
                assert_eq!(
                    Layout::content_top(),
                    Layout::CONTENT_TOP + fonts.line_height(TextStyle::Semibold28) as i32 - 38
                );
                for page in [Page::TextSize, Page::Personalization, Page::Taskbar] {
                    let mut st = settings::Settings::review(&startup, page, false);
                    st.configure(1129, 900);
                    let mut s = Semantics::new(1129, 900);
                    st.semantics(&mut s);
                    let nodes = collect(s);
                    let heading = nodes.iter().find(|n| n.role == Role::Heading).unwrap();
                    // The grown heading line box fits its band and stays above content.
                    assert!(heading.r.3 >= fonts.line_height(TextStyle::Semibold28));
                    assert!(heading.r.1 + heading.r.3 as i32 <= Layout::content_top());
                    let mut checked = 0;
                    for n in nodes.iter().filter(|n| n.target.is_some()) {
                        let (x, y, w, h) = n.r;
                        let bottom = (y + h as i32).min(900);
                        if y < Layout::content_top() && n.role != Role::ListItem {
                            continue;
                        }
                        // Fully visible controls only (rows past the rail or
                        // viewport bottom are offscreen and not hit-testable).
                        if w == 0 || h == 0 || y + h as i32 > 880 || bottom <= y {
                            continue;
                        }
                        let cy = (y + bottom) / 2;
                        let cx = x + w as i32 / 2;
                        assert_eq!(
                            st.target(cx, cy),
                            n.target,
                            "{page:?} {text}% {:?} {:?} {:?}",
                            n.role,
                            n.name,
                            n.r
                        );
                        if n.role == Role::ListItem && n.r.2 == Layout::NAV_W {
                            assert_eq!(h as i32, Layout::nav_row());
                            assert!(line14 <= h, "rail label fits");
                        }
                        checked += 1;
                    }
                    assert!(checked >= 8, "{page:?}: {checked} controls checked");
                    // Painting at this text size never panics and stays opaque.
                    let images = ui::ImageCache::default();
                    let pw = (1129 * display).div_ceil(100);
                    let ph = (900 * display).div_ceil(100);
                    let mut pixels = vec![0; (pw * ph * 4) as usize];
                    let zoom = door_graphics_core::Scale::from_percent(display).unwrap();
                    let mut view = ui::View::new_zoomed(&mut pixels, pw, ph, &fonts, zoom)
                        .with_images(&images);
                    st.draw(&mut view);
                    assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
                }
            }
        },
    );
}

#[test]
fn advanced_pages_move_below_the_grown_heading_and_description() {
    isolated(
        "advanced_pages_move_below_the_grown_heading_and_description",
        || {
            let startup = startup();
            set_text_percent(100);
            let mut base = settings::Settings::review(&startup, Page::Services, false);
            base.configure(1129, 900);
            // A control's 100% hit target moves down by exactly the growth.
            let samples: Vec<_> = (130..600)
                .step_by(3)
                .flat_map(|y| (330..1100).step_by(17).map(move |x| (x, y)))
                .filter_map(|(x, y)| base.target(x, y).filter(|t| *t >= 800).map(|t| (x, y, t)))
                .take(40)
                .collect();
            assert!(!samples.is_empty());
            set_text_percent(225);
            let fonts = FontAtlas::windows_reference_text(100, 225).unwrap();
            let shift = (fonts.line_height(TextStyle::Semibold28) - 38) as i32
                + (fonts.line_height(TextStyle::Regular16) - 24) as i32;
            let mut grown = settings::Settings::review(&startup, Page::Services, false);
            grown.configure(1129, 900 + shift as u32);
            for (x, y, t) in samples {
                assert_eq!(
                    grown.target(x, y + shift),
                    Some(t),
                    "({x}, {y}) moved by {shift}"
                );
            }
            let images = ui::ImageCache::default();
            let mut pixels = vec![0; 1129 * (900 + shift as usize) * 4];
            let mut view = ui::View::new_scaled(&mut pixels, 1129, 900 + shift as u32, &fonts, 1)
                .with_images(&images);
            grown.draw(&mut view);
        },
    );
}
